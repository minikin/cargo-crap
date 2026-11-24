//! Raw verdict files shared by the triage front ends. A read failure is a cache miss.
//! The caller chooses the cache directory and content digest.

mod retention;
pub use retention::Retention;

use crate::cache_file::{read_retrying, write_atomic};
use crate::duplicates::fnv::Fnv1a;
use crate::duplicates::triage::provider::Provider;
use crate::duplicates::triage::verdict::{Kind, Verdict, WorthExtracting};
use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use std::io;
use std::path::{Path, PathBuf};

/// The Rust adapter's legacy cache directory. Other front ends supply a directory to `Settings::for_cache`.
#[must_use]
pub fn default_dir(target_dir: &Path) -> PathBuf {
    target_dir.join("cargo-crap").join("triage")
}

/// A fixed-width content digest. Rust uses its original eight-byte FNV-1a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey<const N: usize = 8>([u8; N]);

impl CacheKey {
    /// Rust's legacy FNV key covers both sources in pair order, the provider namespace,
    /// model and question-set version. `TypeSafe` omits the namespace to reuse pre-provider
    /// entries. Bump the version whenever the question or its scoring changes.
    /// Other front ends can supply their own digest with [`CacheKey::from_digest`].
    #[must_use]
    pub fn new(
        provider: &dyn Provider,
        source_a: &str,
        source_b: &str,
        model: &str,
        version: u32,
    ) -> Self {
        let mut hasher = Fnv1a::default();
        // Length-prefixed, so a boundary moving between fields ("ab" + "c"
        // against "a" + "bc") changes the key.
        let namespace = provider.cache_namespace();
        for field in [source_a, source_b, model].into_iter().chain(namespace) {
            hasher.write(&(field.len() as u64).to_le_bytes());
            hasher.write(field.as_bytes());
        }
        hasher.write(&version.to_le_bytes());
        Self(hasher.finish().to_be_bytes())
    }
}

impl<const N: usize> CacheKey<N> {
    /// A content digest computed by the calling front end.
    #[must_use]
    pub fn from_digest(bytes: [u8; N]) -> Self {
        Self(bytes)
    }

    /// The entry's file name within the cache directory.
    #[must_use]
    pub fn file_name(self) -> String {
        use std::fmt::Write;
        let mut name = String::new();
        for byte in self.0 {
            write!(name, "{byte:02x}").expect("writing to a String");
        }
        name.push_str(".json");
        name
    }
}

/// A verdict cache rooted at one directory.
#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
    retention: Option<Retention>,
}

impl Cache {
    /// A cache in `dir`, which need not exist until the first
    /// [`put`](Self::put).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            retention: None,
        }
    }

    #[must_use]
    pub fn with_retention(
        mut self,
        retention: Retention,
    ) -> Self {
        self.retention = Some(retention);
        self
    }

    /// Remove expired regular verdict files and evict the oldest excess entries.
    ///
    /// # Errors
    /// Returns the first filesystem error other than a concurrent disappearance.
    pub fn prune<const N: usize>(&self) -> io::Result<()> {
        self.retention.map_or(Ok(()), |p| p.prune::<N>(&self.dir))
    }

    /// The verdict stored under `key`, or `None` when there is none or it
    /// cannot be trusted: unreadable, not JSON, or out of range.
    #[must_use]
    pub fn get<const N: usize>(
        &self,
        key: CacheKey<N>,
    ) -> Option<Verdict> {
        let path = self.dir.join(key.file_name());
        if let Some(policy) = self.retention {
            let modified = retention::file_time(&path).ok()??;
            if policy.expired(modified) {
                return None;
            }
        }
        let raw = read_retrying(|| std::fs::read_to_string(&path)).ok()?;
        serde_json::from_str::<Entry>(&raw).ok()?.verdict()
    }

    /// Store `verdict` under `key`, creating the cache directory if needed.
    ///
    /// # Errors
    ///
    /// When the directory cannot be created or the entry cannot be written.
    pub fn put<const N: usize>(
        &self,
        key: CacheKey<N>,
        verdict: &Verdict,
    ) -> io::Result<()> {
        let entry = serde_json::to_string(&Entry::from(verdict)).map_err(io::Error::other)?;
        // Atomic, so a concurrent reader (another worker, or another run
        // sharing target/) sees the old entry or the new one, never a torn
        // file; a read denied access mid-replace is retried in `get`.
        write_atomic(&self.dir, &key.file_name(), entry.as_bytes())
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
    use crate::duplicates::triage::provider::PROVIDERS;
    use crate::duplicates::triage::provider::typesafe::TypeSafe;
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
        CacheKey::new(&TypeSafe, "fn a() {}", "fn b() {}", "jev-latest", 1)
    }

    #[test]
    fn cleanup_ignores_a_vanished_directory_and_reports_an_unreadable_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache");
        let cache = Cache::new(&path).with_retention(Retention {
            now: std::time::SystemTime::now(),
            max_age: std::time::Duration::ZERO,
            max_entries: 0,
        });
        assert!(cache.prune::<32>().is_ok());
        std::fs::write(path, b"file").unwrap();
        assert!(cache.prune::<32>().is_err());
    }

    #[test]
    fn s52_an_expired_digest_is_a_miss_and_cleanup_evicts_the_oldest_tie() {
        use std::{
            fs,
            time::{Duration, UNIX_EPOCH},
        };
        let dir = tempfile::tempdir().unwrap();
        let day = Duration::from_secs(86_400);
        let now = UNIX_EPOCH + day * 100;
        let policy = Retention {
            now,
            max_age: day * 30,
            max_entries: 4096,
        };
        let cache = Cache::new(dir.path()).with_retention(policy);
        let key_for = |i: u32| {
            let mut bytes = [0; 32];
            bytes[28..].copy_from_slice(&i.to_be_bytes());
            CacheKey::from_digest(bytes)
        };
        for n in 0..4098 {
            let key = key_for(n);
            let path = dir.path().join(key.file_name());
            if n < 4 {
                cache.put(key, &verdict()).unwrap();
            } else {
                fs::write(&path, b"{}").unwrap();
            }
            let mtime = match n {
                0 => now - day * 30,
                1 | 2 => now - day * 30 + Duration::from_secs(1),
                _ => now,
            };
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }
        assert_eq!(cache.get(key_for(0)), None);
        assert_eq!(cache.get(key_for(1)), Some(verdict()));
        fs::write(dir.path().join("unrelated.json"), "keep").unwrap();
        fs::write(dir.path().join("temp.tmp"), "keep").unwrap();
        let uppercase = dir.path().join(format!("{}.json", "A".repeat(64)));
        fs::write(&uppercase, "keep").unwrap();
        let wrong_length = dir.path().join(format!("{}.json", "a".repeat(63)));
        fs::write(&wrong_length, "keep").unwrap();
        let directory = dir.path().join(format!("{}.json", "f".repeat(64)));
        fs::create_dir(&directory).unwrap();
        #[cfg(unix)]
        let link = {
            let link = dir.path().join(format!("{}.json", "b".repeat(64)));
            std::os::unix::fs::symlink(dir.path().join(key_for(3).file_name()), &link).unwrap();
            assert_eq!(cache.get(CacheKey::from_digest([0xbb; 32])), None);
            link
        };
        cache.prune::<32>().unwrap();
        assert_eq!(cache.get(key_for(0)), None);
        assert_eq!(cache.get(key_for(1)), None);
        assert_eq!(cache.get(key_for(2)), Some(verdict()));
        assert_eq!(cache.get(key_for(3)), Some(verdict()));
        assert!(!dir.path().join(key_for(0).file_name()).exists());
        assert!(!dir.path().join(key_for(1).file_name()).exists());
        assert_eq!(
            fs::metadata(dir.path().join(key_for(2).file_name()))
                .unwrap()
                .modified()
                .unwrap(),
            now - day * 30 + Duration::from_secs(1)
        );
        assert!(directory.is_dir() && uppercase.is_file() && wrong_length.is_file());
        assert!(dir.path().join("unrelated.json").exists() && dir.path().join("temp.tmp").exists());
        #[cfg(unix)]
        assert!(link.is_symlink());
        let surviving = (0..4098)
            .filter(|&n| dir.path().join(key_for(n).file_name()).exists())
            .count();
        assert_eq!(surviving, 4096);
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
        // The same for a provider with a namespace: renaming its id would
        // silently empty every cache of its verdicts.
        let openai = crate::duplicates::triage::provider::by_id("openai").expect("registered");
        assert_eq!(
            CacheKey::new(openai, "fn a() {}", "fn b() {}", "jev-latest", 1).file_name(),
            "26b95ef93a545bcc.json"
        );
    }

    #[test]
    fn field_boundaries_are_part_of_the_key() {
        assert_ne!(
            CacheKey::new(&TypeSafe, "ab", "c", "m", 1),
            CacheKey::new(&TypeSafe, "a", "bc", "m", 1)
        );
        assert_ne!(
            CacheKey::new(&TypeSafe, "a", "b", "c", 1),
            CacheKey::new(&TypeSafe, "a", "bc", "", 1)
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

    /// The key as it was computed before triage had providers, with its own
    /// FNV-1a rather than the crate's. It is the oracle: `TypeSafe`'s keys
    /// must keep matching it, so no cache is emptied.
    fn key_before_providers(
        source_a: &str,
        source_b: &str,
        model: &str,
        version: u32,
    ) -> CacheKey {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut write = |bytes: &[u8]| {
            for byte in bytes {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        for field in [source_a, source_b, model] {
            write(&(field.len() as u64).to_le_bytes());
            write(field.as_bytes());
        }
        write(&version.to_le_bytes());
        CacheKey(hash.to_be_bytes())
    }

    proptest! {
        /// The same content always yields the same key.
        #[test]
        fn the_key_is_stable(a in ".*", b in ".*", model in ".*", version in any::<u32>()) {
            prop_assert_eq!(
                CacheKey::new(&TypeSafe, &a, &b, &model, version),
                CacheKey::new(&TypeSafe, &a, &b, &model, version)
            );
        }

        /// Changing either body, the model or the question-set version
        /// changes the key: each is part of what the verdict judged.
        #[test]
        fn the_key_changes_with_anything_it_covers(
            a in ".*", b in ".*", model in ".*", version in any::<u32>(), other in ".+",
        ) {
            let base = CacheKey::new(&TypeSafe, &a, &b, &model, version);
            prop_assert_ne!(base, CacheKey::new(&TypeSafe, &format!("{a}{other}"), &b, &model, version));
            prop_assert_ne!(base, CacheKey::new(&TypeSafe, &a, &format!("{b}{other}"), &model, version));
            prop_assert_ne!(base, CacheKey::new(&TypeSafe, &a, &b, &format!("{model}{other}"), version));
            prop_assert_ne!(base, CacheKey::new(&TypeSafe, &a, &b, &model, version.wrapping_add(1)));
        }

        /// `TypeSafe`'s keys are the keys from before providers existed.
        #[test]
        fn the_typesafe_key_is_the_key_from_before_providers(
            a in ".*", b in ".*", model in ".*", version in any::<u32>(),
        ) {
            prop_assert_eq!(
                CacheKey::new(&TypeSafe, &a, &b, &model, version),
                key_before_providers(&a, &b, &model, version)
            );
        }

        /// No two providers share a key for the same bodies and model.
        #[test]
        fn every_provider_keys_its_own_verdicts(
            a in ".*", b in ".*", model in ".*", version in any::<u32>(),
        ) {
            let keys: Vec<CacheKey> = PROVIDERS
                .iter()
                .map(|p| CacheKey::new(*p, &a, &b, &model, version))
                .collect();
            for (i, x) in keys.iter().enumerate() {
                for y in &keys[i + 1..] {
                    prop_assert_ne!(x, y);
                }
            }
        }

        /// A verdict written and read back is the verdict.
        #[test]
        fn write_then_read_returns_the_verdict(verdict in verdicts(), a in ".*", b in ".*") {
            let dir = tempfile::tempdir().expect("temp dir");
            let cache = Cache::new(dir.path());
            let key = CacheKey::new(&TypeSafe, &a, &b, "jev-latest", 1);
            cache.put(key, &verdict).expect("writable");
            prop_assert_eq!(cache.get(key), Some(verdict));
        }
    }
}
