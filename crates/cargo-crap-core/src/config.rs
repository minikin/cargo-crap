//! File lookup for front ends with an optional worktree boundary.

use std::io;
use std::path::{Path, PathBuf};

/// The start directory and the stopping directory both participate in lookup.
pub fn find_upwards(
    start: &Path,
    name: &str,
    stop_at: Option<&Path>,
) -> io::Result<Option<PathBuf>> {
    for directory in start.ancestors() {
        let candidate = directory.join(name);
        if candidate.try_exists()? {
            return Ok(Some(candidate));
        }
        if Some(directory) == stop_at {
            break;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::{fs, io};

    #[test]
    fn an_inclusive_boundary_hides_outer_configs_and_preserves_lookup_errors() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let sub = repo.join("src");
        fs::create_dir_all(&sub).unwrap();
        let name = ".sample.toml";
        fs::write(root.path().join(name), "").unwrap();
        assert_eq!(find_upwards(&sub, name, Some(&repo)).unwrap(), None);
        assert_eq!(
            find_upwards(&sub, name, None).unwrap(),
            Some(root.path().join(name))
        );
        let config = repo.join(name);
        fs::write(&config, "").unwrap();
        assert_eq!(
            find_upwards(&repo, name, Some(&repo)).unwrap(),
            Some(config.clone())
        );
        assert_eq!(
            find_upwards(&config, name, None).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
    }

    proptest! {
        #[test]
        fn nearest_ancestor_wins(mask in 0u8..32, stop in 0usize..5) {
            let root = tempfile::tempdir().unwrap();
            let name = "example.toml";
            let mut dirs = vec![root.path().to_owned()];
            for _ in 1..5 {
                dirs.push(dirs.last().unwrap().join("pkg"));
            }
            fs::create_dir_all(&dirs[4]).unwrap();
            for (i, dir) in dirs.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    fs::write(dir.join(name), "").unwrap();
                }
            }
            let expected = (stop..5).rev().find(|i| mask & (1 << i) != 0).map(|i| dirs[i].join(name));
            prop_assert_eq!(find_upwards(&dirs[4], name, Some(&dirs[stop])).unwrap(), expected);
        }
    }
}
