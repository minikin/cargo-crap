//! The on-disk verdict cache, keyed by the content it judged.
//!
//! One JSON file per pair under `target/cargo-crap/triage/`: gitignored with
//! the rest of `target/`, cleaned by `cargo clean`, machine-local. The key
//! covers both function bodies, the model and the question-set version, so an
//! unchanged pair costs nothing after its first run and a changed body, model
//! or question never reuses an old verdict. A cache that cannot be read is
//! a miss, never an error: the worst it can do is ask the API again.
//!
//! Entries are never evicted: an edited function, a new model or a bumped
//! question set leaves the old entries behind, unread. `cargo clean` is the
//! sweep, as it is for everything else under `target/`.

use crate::duplicates::fingerprint::Fnv1a;
use crate::duplicates::triage::verdict::{Kind, Verdict, WorthExtracting};
use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers the temporary files [`Cache::put`] writes before renaming them.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Where the cache lives under a Cargo target directory.
#[must_use]
pub fn default_dir(target_dir: &Path) -> PathBuf {
    target_dir.join("cargo-crap").join("triage")
}

/// What a cached verdict is keyed by: FNV-1a over both bodies, the model and
/// the question-set version. Stable across processes and toolchains, unlike
/// `DefaultHasher`.
///
/// The request also carries each side's location and the similarity score.
/// A location is context, not content: a function that moved is the same
/// function. Anything else that changes what the model is asked, including
/// how the similarity score is computed, must bump
/// [`QUESTION_SET_VERSION`](super::request::QUESTION_SET_VERSION), or old
/// verdicts are reused for a different question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(u64);

impl CacheKey {
    /// The key for a verdict on `source_a` and `source_b` by `model`, asked
    /// with question-set `version`.
    #[must_use]
    pub fn new(
        source_a: &str,
        source_b: &str,
        model: &str,
        version: u32,
    ) -> Self {
        let mut hasher = Fnv1a::new();
        // Length-prefixed, so a boundary moving between fields ("ab" + "c"
        // against "a" + "bc") changes the key.
        for field in [source_a, source_b, model] {
            hasher.write(&(field.len() as u64).to_le_bytes());
            hasher.write(field.as_bytes());
        }
        hasher.write(&version.to_le_bytes());
        Self(hasher.finish())
    }

    /// The entry's file name within the cache directory.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("{:016x}.json", self.0)
    }
}

/// A verdict cache rooted at one directory.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// A cache in `dir`, which need not exist until the first
    /// [`put`](Self::put).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The verdict stored under `key`, or `None` when there is none or it
    /// cannot be trusted: unreadable, not JSON, or out of range.
    #[must_use]
    pub fn get(
        &self,
        key: CacheKey,
    ) -> Option<Verdict> {
        let raw = std::fs::read_to_string(self.dir.join(key.file_name())).ok()?;
        serde_json::from_str::<Entry>(&raw).ok()?.verdict()
    }

    /// Store `verdict` under `key`, creating the cache directory if needed.
    ///
    /// # Errors
    ///
    /// When the directory cannot be created or the entry cannot be written.
    pub fn put(
        &self,
        key: CacheKey,
        verdict: &Verdict,
    ) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let entry = serde_json::to_string(&Entry::from(verdict)).map_err(io::Error::other)?;
        // Written aside, then renamed into place: a rename within one
        // directory is atomic, so a concurrent reader (another worker, or
        // another run sharing target/) sees the old entry or the new one,
        // never a torn file. The temporary name is unique per process and
        // per write.
        let temp = self.dir.join(format!(
            ".{}.{}.{}.tmp",
            key.file_name(),
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&temp, entry)
            .and_then(|()| std::fs::rename(&temp, self.dir.join(key.file_name())))
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&temp);
            })
    }
}

/// A verdict as stored on disk, checked like an API answer on the way back.
#[derive(Serialize, Deserialize)]
struct Entry {
    kind: String,
    confidence: f64,
    worth_extracting: f64,
    divergence_risk: f64,
}

impl From<&Verdict> for Entry {
    fn from(verdict: &Verdict) -> Self {
        Self {
            kind: verdict.kind.wire_name().to_owned(),
            confidence: verdict.confidence,
            worth_extracting: verdict.worth_extracting.score(),
            divergence_risk: verdict.divergence_risk,
        }
    }
}

impl Entry {
    fn verdict(self) -> Option<Verdict> {
        let unit = |value: f64| (0.0..=1.0).contains(&value).then_some(value);
        Some(Verdict {
            kind: Kind::from_wire(&self.kind)?,
            confidence: unit(self.confidence)?,
            worth_extracting: WorthExtracting::new(self.worth_extracting)?,
            divergence_risk: unit(self.divergence_risk)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::triage::verdict::{Kind, Verdict, WorthExtracting};
    use proptest::prelude::*;
    use std::path::Path;

    fn verdict() -> Verdict {
        Verdict {
            kind: Kind::Parameterisable,
            confidence: 0.83,
            worth_extracting: WorthExtracting::new(2.25).expect("in range"),
            divergence_risk: 0.4,
        }
    }

    fn key() -> CacheKey {
        CacheKey::new("fn a() {}", "fn b() {}", "jev-latest", 1)
    }

    #[test]
    fn a_missing_entry_is_a_miss() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(Cache::new(dir.path()).get(key()), None, "empty cache");
        let nowhere = Cache::new(dir.path().join("never/created"));
        assert_eq!(nowhere.get(key()), None, "no cache directory at all");
    }

    #[test]
    fn a_corrupt_entry_is_a_miss_never_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Cache::new(dir.path());
        let entry = dir.path().join(key().file_name());
        for corrupt in [
            "",
            "not json",
            r#"{"kind":"same_logic"}"#,
            r#"{"kind":"copy_paste","confidence":0.9,"worth_extracting":1.0,"divergence_risk":0.5}"#,
            r#"{"kind":"same_logic","confidence":1.5,"worth_extracting":1.0,"divergence_risk":0.5}"#,
            r#"{"kind":"same_logic","confidence":0.9,"worth_extracting":4.0,"divergence_risk":0.5}"#,
            r#"{"kind":"same_logic","confidence":0.9,"worth_extracting":1.0,"divergence_risk":-0.1}"#,
        ] {
            std::fs::write(&entry, corrupt).expect("write entry");
            assert_eq!(cache.get(key()), None, "treated as a miss: {corrupt:?}");
        }
    }

    #[test]
    fn put_creates_the_cache_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Cache::new(dir.path().join("target/cargo-crap/triage"));
        cache.put(key(), &verdict()).expect("writable");
        assert_eq!(cache.get(key()), Some(verdict()));
    }

    #[test]
    fn a_reader_never_sees_a_half_written_entry() {
        // Rayon workers, or a second run sharing target/, may rewrite an
        // entry while another reads it: a reader sees the old entry or the
        // new one, never a torn file.
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Cache::new(dir.path());
        cache.put(key(), &verdict()).expect("writable");
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..300 {
                        cache.put(key(), &verdict()).expect("writable");
                    }
                });
            }
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..600 {
                        assert_eq!(cache.get(key()), Some(verdict()), "a torn read");
                    }
                });
            }
        });
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("the cache dir")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            [key().file_name()],
            "no temporary file is left behind"
        );
    }

    // TEMPORARY, removed before merge: reproduces the Windows read race on CI
    // and names the error each failed read or write got.
    #[test]
    fn temporary_windows_read_race_diagnosis() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = Cache::new(dir.path());
        cache.put(key(), &verdict()).expect("writable");
        let path = dir.path().join(key().file_name());
        let failures = std::sync::Mutex::new(Vec::new());
        let describe =
            |what: &str, e: &io::Error| format!("{what} {:?} raw={:?}", e.kind(), e.raw_os_error());
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..3000 {
                        if let Err(e) = cache.put(key(), &verdict()) {
                            failures.lock().expect("lock").push(describe("put", &e));
                        }
                    }
                });
            }
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..6000 {
                        match std::fs::read_to_string(&path) {
                            Ok(raw) if serde_json::from_str::<Entry>(&raw).is_err() => {
                                failures.lock().expect("lock").push(format!("torn {raw:?}"));
                            },
                            Ok(_) => {},
                            Err(e) => failures.lock().expect("lock").push(describe("read", &e)),
                        }
                    }
                });
            }
        });
        let failures = failures.into_inner().expect("lock");
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for failure in &failures {
            *counts.entry(failure).or_default() += 1;
        }
        assert!(
            failures.is_empty(),
            "{} failures in 12000 writes and 24000 reads: {counts:?}",
            failures.len()
        );
    }

    #[test]
    fn an_unwritable_cache_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").expect("write");
        assert!(Cache::new(&file).put(key(), &verdict()).is_err());
    }

    #[test]
    fn the_default_directory_is_under_the_target_directory() {
        assert_eq!(
            default_dir(Path::new("target")),
            Path::new("target").join("cargo-crap").join("triage")
        );
    }

    #[test]
    fn a_key_is_the_same_in_every_process() {
        // Pinned: a key that changed with the toolchain or the process would
        // silently empty every cache.
        assert_eq!(key().file_name(), "fb0a17ca1473c168.json");
    }

    #[test]
    fn field_boundaries_are_part_of_the_key() {
        assert_ne!(
            CacheKey::new("ab", "c", "m", 1),
            CacheKey::new("a", "bc", "m", 1)
        );
        assert_ne!(
            CacheKey::new("a", "b", "c", 1),
            CacheKey::new("a", "bc", "", 1)
        );
    }

    fn verdicts() -> impl Strategy<Value = Verdict> {
        (0..Kind::ALL.len(), 0.0..=1.0f64, 0.0..=3.0f64, 0.0..=1.0f64).prop_map(
            |(kind, confidence, worth, divergence_risk)| Verdict {
                kind: Kind::ALL[kind],
                confidence,
                worth_extracting: WorthExtracting::new(worth).expect("in range"),
                divergence_risk,
            },
        )
    }

    proptest! {
        /// The same content always yields the same key.
        #[test]
        fn the_key_is_stable(a in ".*", b in ".*", model in ".*", version in any::<u32>()) {
            prop_assert_eq!(
                CacheKey::new(&a, &b, &model, version),
                CacheKey::new(&a, &b, &model, version)
            );
        }

        /// Changing either body, the model or the question-set version
        /// changes the key: each is part of what the verdict judged.
        #[test]
        fn the_key_changes_with_anything_it_covers(
            a in ".*", b in ".*", model in ".*", version in any::<u32>(), other in ".+",
        ) {
            let base = CacheKey::new(&a, &b, &model, version);
            prop_assert_ne!(base, CacheKey::new(&format!("{a}{other}"), &b, &model, version));
            prop_assert_ne!(base, CacheKey::new(&a, &format!("{b}{other}"), &model, version));
            prop_assert_ne!(base, CacheKey::new(&a, &b, &format!("{model}{other}"), version));
            prop_assert_ne!(base, CacheKey::new(&a, &b, &model, version.wrapping_add(1)));
        }

        /// A verdict written and read back is the verdict.
        #[test]
        fn write_then_read_returns_the_verdict(verdict in verdicts(), a in ".*", b in ".*") {
            let dir = tempfile::tempdir().expect("temp dir");
            let cache = Cache::new(dir.path());
            let key = CacheKey::new(&a, &b, "jev-latest", 1);
            cache.put(key, &verdict).expect("writable");
            prop_assert_eq!(cache.get(key), Some(verdict));
        }
    }
}
