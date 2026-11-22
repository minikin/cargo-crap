//! File lookup for front ends with an optional worktree boundary.

use std::io;
use std::path::{Path, PathBuf};

/// A supplied boundary must be an ancestor in the same lexically normalized form as `start`.
/// Both endpoints participate. Without a boundary the search continues to the root.
pub fn find_upwards(
    start: &Path,
    name: &str,
    stop_at: Option<&Path>,
) -> io::Result<Option<PathBuf>> {
    if let Some(stop) = stop_at
        && !start.starts_with(stop)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "config search boundary is not an ancestor",
        ));
    }
    for directory in start.ancestors() {
        let candidate = directory.join(name);
        match candidate.metadata() {
            Ok(metadata) if metadata.is_file() => return Ok(Some(candidate)),
            Ok(_) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
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
    use std::fs;

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
        #[cfg(unix)]
        assert_eq!(
            find_upwards(&config, name, None).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
    }

    #[test]
    fn a_directory_does_not_shadow_an_outer_file() {
        let fixture = tempfile::tempdir().unwrap();
        let nested = fixture.path().join("nested");
        fs::create_dir_all(nested.join(".config")).unwrap();
        let outer = fixture.path().join(".config");
        fs::write(&outer, "").unwrap();
        assert_eq!(
            find_upwards(&nested, ".config", Some(fixture.path())).unwrap(),
            Some(outer)
        );
    }

    #[test]
    fn a_stop_outside_the_ancestor_chain_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let error =
            find_upwards(root.path(), "missing", Some(&root.path().join("sibling"))).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
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
