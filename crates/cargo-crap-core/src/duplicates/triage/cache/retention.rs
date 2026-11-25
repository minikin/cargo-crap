use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Age and size limits evaluated at the same instant for a whole run.
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub now: SystemTime,
    pub max_age: Duration,
    pub max_entries: usize,
}

impl Retention {
    pub(super) fn expired(
        self,
        modified: SystemTime,
    ) -> bool {
        self.now
            .duration_since(modified)
            .is_ok_and(|age| age >= self.max_age)
    }

    pub(super) fn prune<const N: usize>(
        self,
        dir: &Path,
    ) -> io::Result<()> {
        let Some(entries) = present(fs::read_dir(dir))? else {
            return Ok(());
        };
        self.prune_entries(entries.filter_map(|entry| candidate::<N>(entry).transpose()))
    }

    fn prune_entries(
        self,
        entries: impl Iterator<Item = io::Result<(SystemTime, PathBuf)>>,
    ) -> io::Result<()> {
        let mut error = None;
        let mut files: Vec<_> = entries
            .filter_map(|entry| match entry {
                Ok(file) => Some(file),
                Err(e) => {
                    error.get_or_insert(e);
                    None
                },
            })
            .collect();
        files.sort_unstable();
        let excess = files.len().saturating_sub(self.max_entries);
        for (index, (modified, path)) in files.into_iter().enumerate() {
            if (index < excess || self.expired(modified))
                && let Err(e) = present(fs::remove_file(path))
            {
                error.get_or_insert(e);
            }
        }
        error.map_or(Ok(()), Err)
    }
}

fn candidate<const N: usize>(
    entry: io::Result<fs::DirEntry>
) -> io::Result<Option<(SystemTime, PathBuf)>> {
    let entry = entry?;
    if !digest_name::<N>(&entry.file_name()) {
        return Ok(None);
    }
    let path = entry.path();
    Ok(file_time(&path)?.map(|modified| (modified, path)))
}

fn digest_name<const N: usize>(name: &std::ffi::OsStr) -> bool {
    let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
        return false;
    };
    stem.len() == N * 2
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

pub(super) fn file_time(path: &Path) -> io::Result<Option<SystemTime>> {
    present(fs::symlink_metadata(path))?
        .filter(fs::Metadata::is_file)
        .map(|m| m.modified())
        .transpose()
}

fn present<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_continues_after_read_and_removal_errors_and_returns_the_first_error() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let policy = Retention {
            now,
            max_age: Duration::from_secs(60),
            max_entries: 1,
        };
        let blocked = dir.path().join("a-directory");
        fs::create_dir(&blocked).unwrap();
        let old = dir.path().join("b-file");
        let kept = dir.path().join("c-file");
        fs::write(&kept, []).unwrap();
        for read_error in [false, true] {
            fs::write(&old, []).unwrap();
            let prefix = read_error.then(|| Err(io::Error::other("directory read failed")));
            let entries = [blocked.clone(), old.clone(), kept.clone()].map(|path| Ok((now, path)));
            let error = policy
                .prune_entries(prefix.into_iter().chain(entries))
                .unwrap_err();
            if read_error {
                assert_eq!(error.to_string(), "directory read failed");
            } else {
                assert_eq!(error.kind(), fs::remove_file(&blocked).unwrap_err().kind());
            }
            assert!(!old.exists());
            assert!(kept.is_file());
        }
    }
}
