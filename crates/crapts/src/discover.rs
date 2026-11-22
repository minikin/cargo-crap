use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::InputError;

#[derive(Debug)]
pub struct DiscoveryOptions {
    pub cwd: PathBuf,
}

impl DiscoveryOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }
}

#[derive(Debug)]
pub struct Discovery {
    pub files: Vec<PathBuf>,
}

pub fn discover(
    roots: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<Discovery, InputError> {
    let cwd = options
        .cwd
        .canonicalize()
        .map_err(|error| InputError::at(&options.cwd, error))?;
    let root = worktree(&cwd)?;
    git_files(&cwd, &root, roots)
}

fn worktree(cwd: &Path) -> Result<PathBuf, InputError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-cdup"])
        .output()
        .map_err(|error| InputError::at(cwd, format!("git rev-parse: {error}")))?;
    if !output.status.success() {
        return Err(InputError::at(cwd, String::from_utf8_lossy(&output.stderr)));
    }
    Ok(resolve(
        cwd,
        Path::new(&git_name(output.stdout.trim_ascii_end())),
    ))
}

fn resolve(
    base: &Path,
    path: &Path,
) -> PathBuf {
    let mut result = PathBuf::new();
    for part in base.join(path).components() {
        if part == Component::ParentDir {
            result.pop();
        } else {
            result.push(part);
        }
    }
    result
}

fn git_files(
    cwd: &Path,
    root: &Path,
    roots: &[PathBuf],
) -> Result<Discovery, InputError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args([
            "--literal-pathspecs",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots)
        .output()
        .map_err(|error| InputError::at(cwd, format!("git ls-files: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(InputError::at(
            cwd,
            format!("git ls-files: {}", message.trim_end()),
        ));
    }
    let mut files = BTreeSet::new();
    for record in output.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let name = git_name(record);
        let absolute = resolve(cwd, Path::new(&name));
        if regular_file(&absolute)? && !in_nested_repository(&absolute, root) {
            files.insert(name);
        }
    }
    Ok(Discovery {
        files: files.into_iter().map(PathBuf::from).collect(),
    })
}

fn in_nested_repository(
    file: &Path,
    root: &Path,
) -> bool {
    file.ancestors()
        .skip(1)
        .take_while(|&parent| parent != root)
        .any(|parent| parent.join(".git").exists())
}

fn git_name(bytes: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(bytes).to_owned()
    }
    #[cfg(not(unix))]
    {
        OsString::from(String::from_utf8_lossy(bytes).as_ref())
    }
}

fn regular_file(path: &Path) -> Result<bool, InputError> {
    match fs::symlink_metadata(path) {
        Ok(stat) => Ok(stat.is_file()),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(false)
        },
        Err(error) => Err(InputError::at(path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metadata_error_other_than_absence_retains_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("x".repeat(65536));
        let error = regular_file(&long).unwrap_err();
        assert_eq!(error.path(), long);
        assert_eq!(error.side(), None);
        assert!(error.to_string().contains(&long.display().to_string()));
    }
}
