//! The per-file complexity cache (spec 10): one JSON file,
//! `<target>/cargo-crap/complexity.json`, holding each analysed file's
//! functions keyed by its canonical path and its content.
//!
//! Freshness is the content alone: an entry hits only when the file's length
//! and FNV-1a hash match, whatever its timestamps say. The header names the
//! executable that wrote the cache and the `?` weight it used; any
//! difference discards every entry, so a rebuilt binary never reads scores
//! an older algorithm produced.

use crate::cache::file::{read_retrying, write_atomic};
use crate::complexity::FunctionComplexity;
use crate::duplicates::fingerprint::Fnv1a;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::Hasher;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The cache file's own format. Bump it when [`Raw`] changes shape.
pub const FORMAT: u32 = 1;

/// The version a cache must have been written by to be read.
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The cache file's name under `<target>/cargo-crap/`.
pub const FILE_NAME: &str = "complexity.json";

/// Where the cache lives under a target directory, beside the triage cache.
#[must_use]
pub fn cache_file(target_dir: &Path) -> PathBuf {
    target_dir.join("cargo-crap").join(FILE_NAME)
}

/// The executable that analyses, identified by what a rebuild changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Executable {
    /// Its canonical path, lossily as text.
    pub path: String,
    /// Its length in bytes.
    pub len: u64,
    /// Its modification time, in nanoseconds since the Unix epoch.
    pub mtime_ns: u64,
}

impl Executable {
    /// The running executable, or `None` when it cannot be inspected; the
    /// cache is then off for the run.
    #[must_use]
    pub fn current() -> Option<Self> {
        let path = std::env::current_exe().ok()?.canonicalize().ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        let since_epoch = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Self {
            path: path.to_string_lossy().into_owned(),
            len: meta.len(),
            mtime_ns: u64::try_from(since_epoch.as_nanos()).ok()?,
        })
    }
}

/// What every entry was computed under. A cache whose header differs from
/// the run's is empty to that run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    /// The executable doing the analysis.
    #[serde(rename = "exe")]
    pub executable: Executable,
    /// The `?` weight (spec 27), compared bit for bit.
    pub try_weight: f64,
}

/// One function as stored: everything but the file, which the walk supplies.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFunction {
    name: String,
    start_line: usize,
    end_line: usize,
    cyclomatic: f64,
}

/// One file's entry: its content key and its functions.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    len: u64,
    hash: String,
    functions: Vec<CachedFunction>,
}

impl Header {
    /// Whether entries written under `self` hold for a run under `other`.
    fn same_as(
        &self,
        other: &Self,
    ) -> bool {
        self.executable == other.executable
            && self.try_weight.to_bits() == other.try_weight.to_bits()
    }
}

/// The file as written.
#[derive(Serialize, Deserialize)]
struct Raw {
    format: u32,
    crate_version: String,
    #[serde(flatten)]
    header: Header,
    files: BTreeMap<String, Entry>,
}

/// A cache hit: the functions, and the record that carries the file into
/// the next save without hashing it again.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The file's functions, each carrying the walked path.
    pub functions: Vec<FunctionComplexity>,
    /// The entry, to store again.
    pub record: FileRecord,
}

/// A file this run analysed, ready to be stored.
#[derive(Debug, Clone)]
pub struct FileRecord {
    /// The file's canonical path.
    pub key: PathBuf,
    entry: Entry,
}

impl FileRecord {
    /// A record of `functions`, found in a file at `key` holding `bytes`.
    #[must_use]
    pub fn new(
        key: PathBuf,
        bytes: &[u8],
        functions: &[FunctionComplexity],
    ) -> Self {
        let functions = functions
            .iter()
            .map(|f| CachedFunction {
                name: f.name.clone(),
                start_line: f.start_line,
                end_line: f.end_line,
                cyclomatic: f.cyclomatic,
            })
            .collect();
        Self {
            key,
            entry: Entry {
                len: bytes.len() as u64,
                hash: content_hash(bytes),
                functions,
            },
        }
    }
}

/// FNV-1a over `bytes`, as 16 hex digits.
fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Fnv1a::new();
    hasher.write(bytes);
    format!("{:016x}", hasher.finish())
}

/// The entries a run may use, under the header it runs with.
#[derive(Debug, Clone)]
pub struct ComplexityCache {
    header: Header,
    entries: BTreeMap<String, Entry>,
}

impl ComplexityCache {
    /// The `?` weight every entry was computed with, and every miss must be.
    #[must_use]
    pub fn try_weight(&self) -> f64 {
        self.header.try_weight
    }

    /// A cache with no entries.
    #[must_use]
    pub fn empty(header: Header) -> Self {
        Self {
            header,
            entries: BTreeMap::new(),
        }
    }

    /// The cache in `file`, or an empty one when the file is missing, is not
    /// a cache, or was written under a different header.
    #[must_use]
    pub fn load(
        file: &Path,
        header: Header,
    ) -> Self {
        let raw = read_retrying(|| std::fs::read(file))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Raw>(&bytes).ok())
            .filter(|raw| raw.matches(&header));
        Self {
            entries: raw.map(|raw| raw.files).unwrap_or_default(),
            header,
        }
    }

    /// The functions stored for the file at canonical path `key`, if its
    /// content is still `bytes`, each carrying `walked` as its file.
    #[must_use]
    pub fn lookup(
        &self,
        key: &Path,
        bytes: &[u8],
        walked: &Path,
    ) -> Option<Hit> {
        let entry = self.entries.get(key.to_str()?)?;
        if entry.len != bytes.len() as u64 || entry.hash != content_hash(bytes) {
            return None;
        }
        let functions = entry
            .functions
            .iter()
            .map(|f| FunctionComplexity {
                file: walked.to_path_buf(),
                name: f.name.clone(),
                start_line: f.start_line,
                end_line: f.end_line,
                cyclomatic: f.cyclomatic,
            })
            .collect();
        let record = FileRecord {
            key: key.to_path_buf(),
            entry: entry.clone(),
        };
        Some(Hit { functions, record })
    }

    /// Write the cache to `file`: the entries outside every walked root as
    /// they were, and `records` in place of everything under them. Roots are
    /// canonicalized here, as the keys are, so `.` evicts what it walked. A
    /// record whose path is not UTF-8 is left out.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save(
        self,
        file: &Path,
        walked_roots: &[PathBuf],
        records: Vec<FileRecord>,
    ) -> io::Result<()> {
        let roots: Vec<PathBuf> = walked_roots
            .iter()
            .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
            .collect();
        let mut files: BTreeMap<String, Entry> = self
            .entries
            .into_iter()
            .filter(|(key, _)| !roots.iter().any(|root| Path::new(key).starts_with(root)))
            .collect();
        for record in records {
            if let Some(key) = record.key.to_str() {
                files.insert(key.to_owned(), record.entry);
            }
        }
        let raw = Raw {
            format: FORMAT,
            crate_version: CRATE_VERSION.to_owned(),
            header: self.header,
            files,
        };
        let bytes = serde_json::to_vec(&raw).map_err(io::Error::other)?;
        let (dir, name) = (file.parent(), file.file_name().and_then(|n| n.to_str()));
        match (dir, name) {
            (Some(dir), Some(name)) => write_atomic(dir, name, &bytes),
            _ => Err(io::Error::other("the cache file has no directory or name")),
        }
    }
}

/// One run's cache: the file it lives in and what it held when opened.
#[derive(Debug)]
pub struct Session {
    file: PathBuf,
    cache: ComplexityCache,
}

impl Session {
    /// The cache under `target_dir` for a run with `try_weight`, or `None`
    /// when the running executable cannot be identified.
    #[must_use]
    pub fn open(
        target_dir: &Path,
        try_weight: f64,
    ) -> Option<Self> {
        let header = Header {
            executable: Executable::current()?,
            try_weight,
        };
        let file = cache_file(target_dir);
        let cache = ComplexityCache::load(&file, header);
        Some(Self { file, cache })
    }

    /// The entries the run may use.
    #[must_use]
    pub fn cache(&self) -> &ComplexityCache {
        &self.cache
    }

    /// Save what the run analysed under `walked_roots`. A cache that cannot
    /// be written is skipped silently: the next run parses again, which is
    /// all a missing cache costs.
    pub fn save(
        self,
        walked_roots: &[PathBuf],
        records: Vec<FileRecord>,
    ) {
        let _ = self.cache.save(&self.file, walked_roots, records);
    }
}

impl Raw {
    /// Whether this file was written by `header`'s executable and weight,
    /// in this format by this version.
    fn matches(
        &self,
        header: &Header,
    ) -> bool {
        self.format == FORMAT && self.crate_version == CRATE_VERSION && self.header.same_as(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::{Path, PathBuf};

    /// A file's bytes, kept out of `prop_assert!`, which reads braces as format arguments.
    const SOURCE: &[u8] = b"fn a() {}";

    fn header() -> Header {
        Header {
            executable: Executable {
                path: "/bin/cargo-crap".to_owned(),
                len: 1000,
                mtime_ns: 42,
            },
            try_weight: 1.0,
        }
    }

    fn function(
        name: &str,
        cc: f64,
    ) -> FunctionComplexity {
        FunctionComplexity {
            file: PathBuf::from("ignored"),
            name: name.to_owned(),
            start_line: 3,
            end_line: 9,
            cyclomatic: cc,
        }
    }

    /// The functions `lookup` returned, as (name, start, end, cc, file).
    fn shape(fns: &[FunctionComplexity]) -> Vec<(String, usize, usize, u64, PathBuf)> {
        fns.iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.start_line,
                    f.end_line,
                    f.cyclomatic.to_bits(),
                    f.file.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn a_missing_empty_or_corrupt_file_loads_an_empty_cache() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let key = Path::new("/p/src/lib.rs");
        let missing = ComplexityCache::load(&file, header());
        assert!(missing.lookup(key, b"fn a() {}", key).is_none());
        for corrupt in ["", "not json", "{}", r#"{"format":1}"#, "\u{0}\u{1}"] {
            std::fs::write(&file, corrupt).expect("write");
            let cache = ComplexityCache::load(&file, header());
            assert!(
                cache.lookup(key, b"fn a() {}", key).is_none(),
                "{corrupt:?}"
            );
        }
    }

    #[test]
    fn a_hit_carries_the_path_the_caller_walked_not_the_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let key = PathBuf::from("/p/src/lib.rs");
        let bytes = b"fn a() { if x {} }";
        let record = FileRecord::new(key.clone(), bytes, &[function("a", 2.0)]);
        ComplexityCache::empty(header())
            .save(&file, &[PathBuf::from("/p")], vec![record])
            .expect("writable");
        let cache = ComplexityCache::load(&file, header());
        let walked = Path::new("./src/lib.rs");
        let hit = cache.lookup(&key, bytes, walked).expect("a hit");
        assert_eq!(
            shape(&hit.functions),
            [("a".to_owned(), 3, 9, 2.0f64.to_bits(), walked.to_path_buf())]
        );
    }

    #[test]
    fn a_file_with_no_functions_is_a_hit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let key = PathBuf::from("/p/src/empty.rs");
        let record = FileRecord::new(key.clone(), b"// nothing", &[]);
        ComplexityCache::empty(header())
            .save(&file, &[PathBuf::from("/p")], vec![record])
            .expect("writable");
        let cache = ComplexityCache::load(&file, header());
        assert_eq!(
            cache
                .lookup(&key, b"// nothing", &key)
                .map(|h| h.functions.len()),
            Some(0)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_never_stored_and_the_rest_are() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/p/src/\xff.rs"));
        let utf8 = PathBuf::from("/p/src/lib.rs");
        let records = vec![
            FileRecord::new(odd.clone(), b"fn a() {}", &[function("a", 1.0)]),
            FileRecord::new(utf8.clone(), b"fn b() {}", &[function("b", 1.0)]),
        ];
        ComplexityCache::empty(header())
            .save(&file, &[PathBuf::from("/p")], records)
            .expect("one odd path does not fail the save");
        let cache = ComplexityCache::load(&file, header());
        assert!(cache.lookup(&odd, b"fn a() {}", &odd).is_none());
        assert!(cache.lookup(&utf8, b"fn b() {}", &utf8).is_some());
    }

    #[test]
    fn a_cache_written_under_another_header_loads_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let key = PathBuf::from("/p/src/lib.rs");
        let exe = header().executable;
        let others = [
            Header {
                try_weight: 0.5,
                ..header()
            },
            Header {
                executable: Executable {
                    len: 1001,
                    ..exe.clone()
                },
                ..header()
            },
            Header {
                executable: Executable {
                    mtime_ns: 43,
                    ..exe.clone()
                },
                ..header()
            },
            Header {
                executable: Executable {
                    path: "/other".to_owned(),
                    ..exe
                },
                ..header()
            },
        ];
        for other in others {
            let record = FileRecord::new(key.clone(), SOURCE, &[function("a", 1.0)]);
            ComplexityCache::empty(header())
                .save(&file, &[PathBuf::from("/p")], vec![record])
                .expect("writable");
            let cache = ComplexityCache::load(&file, other.clone());
            assert!(cache.lookup(&key, SOURCE, &key).is_none(), "{other:?}");
        }
    }

    #[test]
    fn a_hits_record_stores_the_same_entry_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join(FILE_NAME);
        let key = PathBuf::from("/p/src/lib.rs");
        let record = FileRecord::new(key.clone(), SOURCE, &[function("a", 2.5)]);
        ComplexityCache::empty(header())
            .save(&file, &[PathBuf::from("/p")], vec![record])
            .expect("writable");
        let hit = ComplexityCache::load(&file, header())
            .lookup(&key, SOURCE, &key)
            .expect("a hit");
        ComplexityCache::load(&file, header())
            .save(&file, &[PathBuf::from("/p")], vec![hit.record])
            .expect("writable");
        let again = ComplexityCache::load(&file, header()).lookup(&key, SOURCE, &key);
        assert_eq!(
            shape(&again.expect("still a hit").functions),
            shape(&hit.functions)
        );
    }

    #[test]
    fn a_root_spelled_another_way_still_evicts_what_it_walked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().canonicalize().expect("canonical");
        std::fs::create_dir_all(root.join("sub")).expect("mkdir");
        let file = root.join(FILE_NAME);
        let gone = root.join("gone.rs");
        let record = FileRecord::new(gone.clone(), SOURCE, &[function("a", 1.0)]);
        ComplexityCache::empty(header())
            .save(&file, std::slice::from_ref(&root), vec![record])
            .expect("writable");
        // The next run walks the same root as `sub/..` and finds nothing.
        ComplexityCache::load(&file, header())
            .save(&file, &[root.join("sub/..")], vec![])
            .expect("writable");
        let cache = ComplexityCache::load(&file, header());
        assert!(cache.lookup(&gone, SOURCE, &gone).is_none(), "evicted");
    }

    #[test]
    fn a_session_saves_where_the_next_one_reads() {
        let target = tempfile::tempdir().expect("temp dir");
        let key = PathBuf::from("/p/src/lib.rs");
        let session = Session::open(target.path(), 1.0).expect("the executable is known");
        assert!(session.cache().lookup(&key, SOURCE, &key).is_none(), "cold");
        let record = FileRecord::new(key.clone(), SOURCE, &[function("a", 1.0)]);
        session.save(&[PathBuf::from("/p")], vec![record]);
        assert!(cache_file(target.path()).exists());
        let next = Session::open(target.path(), 1.0).expect("the executable is known");
        assert!(next.cache().lookup(&key, SOURCE, &key).is_some(), "warm");
        let other_weight = Session::open(target.path(), 0.5).expect("the executable is known");
        assert!(
            other_weight.cache().lookup(&key, SOURCE, &key).is_none(),
            "another weight"
        );
    }

    #[test]
    fn a_session_that_cannot_save_is_silent() {
        let dir = tempfile::tempdir().expect("temp dir");
        // `cargo-crap` is a file, so the cache directory cannot be created.
        std::fs::write(dir.path().join("cargo-crap"), b"").expect("write");
        let session = Session::open(dir.path(), 1.0).expect("the executable is known");
        session.save(&[PathBuf::from("/p")], vec![]);
        assert!(!cache_file(dir.path()).exists());
    }

    #[test]
    fn the_running_executable_is_identified_by_its_file() {
        let exe = Executable::current().expect("the test binary can be inspected");
        let on_disk = std::fs::metadata(&exe.path).expect("the path names a file");
        assert_eq!(exe.len, on_disk.len());
        assert!(exe.len > 0 && exe.mtime_ns > 0, "{exe:?}");
        assert_eq!(Executable::current(), Some(exe), "stable within a run");
    }

    #[test]
    fn the_cache_lives_beside_the_triage_cache() {
        assert_eq!(
            cache_file(Path::new("/t")),
            Path::new("/t/cargo-crap/complexity.json")
        );
    }

    /// A generated file: a name, its bytes, and its functions' names and CCs.
    type Generated = (String, Vec<u8>, Vec<(String, f64)>);

    fn records() -> impl Strategy<Value = Vec<Generated>> {
        proptest::collection::vec(
            (
                "[a-z]{1,8}",
                proptest::collection::vec(any::<u8>(), 0..64),
                proptest::collection::vec(("[a-z_]{1,10}", 1.0f64..50.0), 0..4),
            ),
            0..6,
        )
    }

    fn build(
        root: &str,
        generated: &[Generated],
    ) -> Vec<FileRecord> {
        generated
            .iter()
            .enumerate()
            .map(|(i, (name, bytes, fns))| {
                let fns: Vec<_> = fns.iter().map(|(n, cc)| function(n, *cc)).collect();
                FileRecord::new(PathBuf::from(format!("{root}/{i}_{name}.rs")), bytes, &fns)
            })
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn save_then_load_returns_every_entry(generated in records()) {
            let dir = tempfile::tempdir().expect("temp dir");
            let file = dir.path().join(FILE_NAME);
            let records = build("/p", &generated);
            ComplexityCache::empty(header())
                .save(&file, &[PathBuf::from("/p")], records)
                .expect("writable");
            let cache = ComplexityCache::load(&file, header());
            for (record, (_, bytes, fns)) in build("/p", &generated).iter().zip(&generated) {
                let hit = cache.lookup(&record.key, bytes, &record.key).expect("a hit");
                let expected: Vec<_> = fns.iter().map(|(n, cc)| {
                    (n.clone(), 3, 9, cc.to_bits(), record.key.clone())
                }).collect();
                prop_assert_eq!(shape(&hit.functions), expected);
            }
        }

        #[test]
        fn any_changed_byte_is_a_miss(
            bytes in proptest::collection::vec(any::<u8>(), 1..128),
            at in any::<prop::sample::Index>(),
            flip in 1u8..=255,
            extra in proptest::collection::vec(any::<u8>(), 1..4),
        ) {
            let dir = tempfile::tempdir().expect("temp dir");
            let file = dir.path().join(FILE_NAME);
            let key = PathBuf::from("/p/src/lib.rs");
            let record = FileRecord::new(key.clone(), &bytes, &[function("a", 1.0)]);
            ComplexityCache::empty(header())
                .save(&file, &[PathBuf::from("/p")], vec![record])
                .expect("writable");
            let cache = ComplexityCache::load(&file, header());
            prop_assert!(cache.lookup(&key, &bytes, &key).is_some());
            let mut same_length = bytes.clone();
            same_length[at.index(bytes.len())] ^= flip;
            prop_assert!(cache.lookup(&key, &same_length, &key).is_none());
            let mut longer = bytes.clone();
            longer.extend(&extra);
            prop_assert!(cache.lookup(&key, &longer, &key).is_none());
        }

        #[test]
        fn a_different_header_misses_every_lookup(field in 0usize..6) {
            let dir = tempfile::tempdir().expect("temp dir");
            let file = dir.path().join(FILE_NAME);
            let key = PathBuf::from("/p/src/lib.rs");
            let record = FileRecord::new(key.clone(), SOURCE, &[function("a", 1.0)]);
            ComplexityCache::empty(header())
                .save(&file, &[PathBuf::from("/p")], vec![record])
                .expect("writable");
            let mut raw: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&file).expect("read")).expect("json");
            match field {
                0 => raw["format"] = serde_json::json!(FORMAT + 1),
                1 => raw["crate_version"] = serde_json::json!("0.0.0-other"),
                2 => raw["exe"]["path"] = serde_json::json!("/other/cargo-crap"),
                3 => raw["exe"]["len"] = serde_json::json!(1001),
                4 => raw["exe"]["mtime_ns"] = serde_json::json!(43),
                _ => raw["try_weight"] = serde_json::json!(0.5),
            }
            std::fs::write(&file, raw.to_string()).expect("write");
            let cache = ComplexityCache::load(&file, header());
            prop_assert!(cache.lookup(&key, SOURCE, &key).is_none());
        }

        #[test]
        fn a_save_replaces_only_the_walked_roots(
            old_a in records(),
            old_b in records(),
            new_a in records(),
        ) {
            let dir = tempfile::tempdir().expect("temp dir");
            let file = dir.path().join(FILE_NAME);
            let (a, b) = (PathBuf::from("/ws/a"), PathBuf::from("/ws/b"));
            let mut first = build("/ws/a", &old_a);
            first.extend(build("/ws/b", &old_b));
            ComplexityCache::empty(header())
                .save(&file, &[a.clone(), b.clone()], first)
                .expect("writable");
            // A run that walked only /ws/a, finding new_a there.
            ComplexityCache::load(&file, header())
                .save(&file, std::slice::from_ref(&a), build("/ws/a/new", &new_a))
                .expect("writable");
            let cache = ComplexityCache::load(&file, header());
            for (record, (_, bytes, _)) in build("/ws/b", &old_b).iter().zip(&old_b) {
                prop_assert!(cache.lookup(&record.key, bytes, &record.key).is_some(), "kept");
            }
            for (record, (_, bytes, _)) in build("/ws/a", &old_a).iter().zip(&old_a) {
                prop_assert!(cache.lookup(&record.key, bytes, &record.key).is_none(), "evicted");
            }
            for (record, (_, bytes, _)) in build("/ws/a/new", &new_a).iter().zip(&new_a) {
                prop_assert!(cache.lookup(&record.key, bytes, &record.key).is_some(), "stored");
            }
        }
    }
}
