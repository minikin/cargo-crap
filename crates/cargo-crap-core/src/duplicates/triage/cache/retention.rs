use std::{
    fs, io,
    path::Path,
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
        let mut survivors = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !digest_name::<N>(&entry.file_name()) {
                continue;
            }
            let path = entry.path();
            let Some(modified) = file_time(&path)? else {
                continue;
            };
            if self.expired(modified) {
                present(fs::remove_file(path))?;
            } else {
                survivors.push((modified, path));
            }
        }
        survivors.sort_unstable();
        let excess = survivors.len().saturating_sub(self.max_entries);
        for (_, path) in survivors.into_iter().take(excess) {
            present(fs::remove_file(path))?;
        }
        Ok(())
    }
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
