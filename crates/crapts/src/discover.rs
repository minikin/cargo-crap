use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
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
    let repository = worktree(&cwd)?;
    let mut inputs = Vec::new();
    for root in roots {
        let absolute = absolute_input(&options.cwd, &cwd, root)?;
        if !through_symlink(&absolute, repository.as_deref().unwrap_or(&cwd)) {
            inputs.push(absolute);
        }
    }
    if roots.is_empty() {
        inputs.push(cwd.clone());
    }
    let names = match repository {
        Some(root) if !inputs.is_empty() => git_files(&cwd, &root, &inputs)?,
        Some(_) => BTreeSet::new(),
        None => walk_files(&cwd, inputs)?,
    };
    Ok(Discovery {
        files: names.into_iter().map(PathBuf::from).collect(),
    })
}

fn worktree(cwd: &Path) -> Result<Option<PathBuf>, InputError> {
    let output = match Command::new("git")
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .args(["rev-parse", "--is-inside-work-tree", "--show-cdup"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(InputError::at(cwd, format!("git rev-parse: {error}"))),
    };
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if message.contains("not a git repository") {
            return Ok(None);
        }
        return Err(InputError::at(
            cwd,
            format!("git rev-parse: {}", message.trim_end()),
        ));
    }
    let response = String::from_utf8_lossy(&output.stdout);
    let mut lines = response.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    Ok(Some(resolve(
        cwd,
        Path::new(lines.next().unwrap_or_default()),
    )))
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
) -> Result<BTreeSet<OsString>, InputError> {
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
        .args(roots.iter().map(|root| relative(cwd, root)))
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
        if !through_symlink(&absolute, root)
            && !in_nested_repository(&absolute, root)
            && regular_file(&absolute)?
        {
            files.insert(name);
        }
    }
    Ok(files)
}

fn absolute_input(
    invoked: &Path,
    cwd: &Path,
    input: &Path,
) -> Result<PathBuf, InputError> {
    if !input.is_absolute() {
        return Ok(resolve(cwd, input));
    }
    let logical = std::path::absolute(invoked).map_err(|e| InputError::at(invoked, e))?;
    let Some(common) = logical.ancestors().find(|dir| input.starts_with(dir)) else {
        return Ok(input.to_owned());
    };
    let physical = common
        .canonicalize()
        .map_err(|e| InputError::at(common, e))?;
    Ok(resolve(&physical, input.strip_prefix(common).unwrap()))
}

fn relative(
    cwd: &Path,
    path: &Path,
) -> OsString {
    let Some(common) = cwd.ancestors().find(|dir| path.starts_with(dir)) else {
        return path.as_os_str().to_owned();
    };
    let up = cwd
        .strip_prefix(common)
        .unwrap()
        .iter()
        .map(|_| OsStr::new(".."));
    let down = path.strip_prefix(common).unwrap().iter();
    let parts: Vec<_> = up.chain(down).collect();
    if parts.is_empty() {
        OsString::from(".")
    } else {
        parts.join(OsStr::new("/"))
    }
}

fn through_symlink(
    path: &Path,
    boundary: &Path,
) -> bool {
    path.ancestors()
        .take_while(|part| !boundary.starts_with(part))
        .any(Path::is_symlink)
}

fn pruned(name: &OsStr) -> bool {
    name == ".git" || name == "node_modules"
}

fn walk_files(
    cwd: &Path,
    roots: Vec<PathBuf>,
) -> Result<BTreeSet<OsString>, InputError> {
    let mut pending: Vec<_> = roots
        .into_iter()
        .filter(|root| !Path::new(&relative(cwd, root)).iter().any(pruned))
        .collect();
    let mut files = BTreeSet::new();
    while let Some(path) = pending.pop() {
        let kind = fs::symlink_metadata(&path)
            .map_err(|e| InputError::at(&path, e))?
            .file_type();
        if kind.is_file() {
            files.insert(relative(cwd, &path));
        } else if kind.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| InputError::at(&path, e))? {
                let entry = entry.map_err(|e| InputError::at(&path, e))?;
                if !pruned(&entry.file_name()) {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(files)
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

    #[cfg(windows)]
    #[test]
    fn a_root_on_another_drive_keeps_its_absolute_path() {
        let cwd = Path::new("C:/pkg");
        let file = Path::new("D:/shared/a.ts");
        assert_eq!(absolute_input(cwd, cwd, file).unwrap(), file);
        assert_eq!(relative(cwd, file), file.as_os_str());
    }

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
