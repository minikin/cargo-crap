use cargo_crap_core::duplicates::triage::cache::{Cache, Retention};
use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(super) fn resolve(
    os: &str,
    boundary: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    now: SystemTime,
) -> io::Result<Cache> {
    let absolute = |name| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let root = match os {
        "macos" => absolute("HOME").map(|p| p.join("Library/Caches")),
        "windows" => absolute("LOCALAPPDATA"),
        _ => absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|p| p.join(".cache"))),
    }
    .ok_or_else(|| io::Error::other("no absolute per-user cache directory"))?;
    let dir = physical(&root.join("crapts/triage"))?;
    if dir.starts_with(boundary.canonicalize()?) {
        return Err(io::Error::other(
            "the per-user cache resolves inside the worktree",
        ));
    }
    Ok(Cache::new(dir).with_retention(Retention {
        now,
        max_age: Duration::from_hours(30 * 24),
        max_entries: 4096,
    }))
}

// Resolve existing links before interpreting a later parent component. Missing
// components are allowed because the cache is created only on its first write.
fn physical(path: &Path) -> io::Result<PathBuf> {
    let root = path
        .ancestors()
        .last()
        .ok_or_else(|| io::Error::other("missing cache root"))?;
    let mut resolved = root.canonicalize()?;
    for component in path
        .strip_prefix(root)
        .map_err(io::Error::other)?
        .components()
    {
        match component {
            Component::ParentDir => {
                resolved.pop();
            },
            Component::Normal(name) => {
                resolved.push(name);
                match fs::symlink_metadata(&resolved) {
                    Ok(_) => {
                        resolved = resolved.canonicalize()?;
                        // On Windows, looking up a child beneath a regular file returns NotFound
                        // instead of NotADirectory, so reject non-directory prefixes here.
                        if !resolved.is_dir() {
                            return Err(io::Error::other("cache path traverses a non-directory"));
                        }
                    },
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            },
            _ => (),
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cargo_crap_core::duplicates::{
        triage::cache::CacheKey,
        verdict::{Kind, Verdict, WorthExtracting},
    };
    fn put(cache: &Cache) {
        cache
            .put(
                CacheKey::from_digest([0; 32]),
                &Verdict {
                    kind: Kind::SameLogic,
                    confidence: 1.0,
                    worth_extracting: WorthExtracting::new(3.0).unwrap(),
                    divergence_risk: 0.5,
                },
            )
            .unwrap();
    }
    #[test]
    fn s33_os_cache_roots_are_absolute_lazy_and_outside_the_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        fs::create_dir(&project).unwrap();
        for (n, (os, key, sub)) in [
            ("linux", "XDG_CACHE_HOME", ""),
            ("linux", "HOME", ".cache"),
            ("macos", "HOME", "Library/Caches"),
            ("windows", "LOCALAPPDATA", ""),
        ]
        .into_iter()
        .enumerate()
        {
            let root = temp.path().join(format!("user{n}"));
            let cache = resolve(
                os,
                &project,
                |name| {
                    if name == key {
                        Some(root.clone().into_os_string())
                    } else {
                        Some("relative".into())
                    }
                },
                SystemTime::now(),
            )
            .unwrap();
            assert!(!root.exists());
            put(&cache);
            assert_eq!(
                fs::read_dir(root.join(sub).join("crapts/triage"))
                    .unwrap()
                    .count(),
                1
            );
        }
        for os in ["linux", "macos", "windows"] {
            assert!(resolve(os, &project, |_| Some("relative".into()), SystemTime::now()).is_err());
            assert!(
                resolve(
                    os,
                    &project,
                    |_| Some(project.clone().into_os_string()),
                    SystemTime::now()
                )
                .is_err()
            );
        }
        assert_eq!(fs::read_dir(project).unwrap().count(), 0);
    }

    #[test]
    fn s52_the_frontend_uses_thirty_calendar_days_before_cleanup() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("project");
        fs::create_dir(&repo).unwrap();
        let now = SystemTime::now();
        let cache = resolve(
            "linux",
            &repo,
            |_| Some(d.path().as_os_str().to_owned()),
            now,
        )
        .unwrap();
        put(&cache);
        let key = CacheKey::from_digest([0; 32]);
        let file = fs::File::options()
            .write(true)
            .open(d.path().join("crapts/triage").join(key.file_name()))
            .unwrap();
        file.set_modified(now - Duration::from_hours(24 * 29))
            .unwrap();
        assert!(cache.get(key).is_some());
        file.set_modified(now - Duration::from_hours(24 * 31))
            .unwrap();
        assert!(cache.get(key).is_none());
    }

    #[test]
    fn a_regular_file_blocks_the_cache_at_any_depth() {
        for blocked in ["user", "user/crapts", "user/crapts/triage"] {
            let temp = tempfile::tempdir().unwrap();
            let file = temp.path().join(blocked);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "").unwrap();
            assert!(
                physical(&temp.path().join("user/crapts/triage")).is_err(),
                "{blocked}"
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_directory_link_can_contain_the_cache() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("cache");
        let link = temp.path().join("alias");
        fs::create_dir(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_dir(&target, &link) {
            // Windows requires developer mode or the create-symbolic-links privilege.
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("creating test symlink: {error}");
        }
        assert_eq!(
            physical(&link.join("crapts/triage")).unwrap(),
            target.canonicalize().unwrap().join("crapts/triage")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_the_repository_cannot_become_the_cache_root() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        fs::create_dir(&project).unwrap();
        std::os::unix::fs::symlink(&project, temp.path().join("link")).unwrap();
        for root in [temp.path().join("link"), temp.path().join("new/../link")] {
            assert!(
                resolve(
                    "linux",
                    &project,
                    |_| Some(root.clone().into_os_string()),
                    SystemTime::now()
                )
                .is_err()
            );
        }
        assert!(fs::read_dir(&project).unwrap().next().is_none());
    }
}
