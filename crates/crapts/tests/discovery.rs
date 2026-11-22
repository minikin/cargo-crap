use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crapts::{DiscoveryOptions, discover};
use tempfile::tempdir;

fn git(
    cwd: &Path,
    args: &[&str],
) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(
    root: &Path,
    path: &str,
    contents: &str,
) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, contents).unwrap();
}

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

fn files(
    cwd: &Path,
    roots: &[PathBuf],
) -> Vec<String> {
    discover(roots, &DiscoveryOptions::new(cwd))
        .unwrap()
        .files
        .into_iter()
        .map(|p| p.into_os_string().into_string().unwrap())
        .collect()
}

#[test]
fn discover_the_ordinary_git_file_set() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    for file in [
        "src/a.ts",
        "src/gone.ts",
        "src/b.js",
        "src/tmp.js",
        "src/embedded/child.ts",
        "src/sub/child.ts",
    ] {
        write(root, file, "");
    }
    git(root, &["add", "src/a.ts", "src/gone.ts"]);
    write(root, "src/replaced/old.ts", "");
    git(root, &["add", "src/replaced/old.ts"]);
    fs::remove_file(root.join("src/replaced/old.ts")).unwrap();
    fs::remove_dir(root.join("src/replaced")).unwrap();
    write(root, "src/replaced", "");
    fs::remove_file(root.join("src/gone.ts")).unwrap();
    write(root, ".gitignore", "src/tmp.js\nsrc/a.ts\nsrc/replaced\n");
    write(root, ".gitattributes", "src/a.ts linguist-generated=true\n");
    git(&root.join("src/embedded"), &["init", "-q"]);
    write(root, "src/vendor/old.ts", "");
    git(root, &["add", "src/vendor/old.ts"]);
    git(&root.join("src/vendor"), &["init", "-q"]);
    git(
        root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1234567890123456789012345678901234567890,src/sub",
        ],
    );
    let expected = ["src/a.ts", "src/b.js"];
    assert_eq!(files(root, &paths(&["src", "src/a.ts"])), expected);
    assert_eq!(files(root, &paths(&["src/a.ts", "src", "src"])), expected);
    assert_eq!(
        files(root, &paths(&["src/vendor/old.ts"])),
        Vec::<String>::new()
    );
}

#[test]
fn resolve_input_roots_and_preserve_path_spelling() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    for file in ["pkg/[x]/a b.ts", "pkg/x/a b.ts", "shared/café.ts"] {
        write(root, file, "");
    }
    let cwd = root.join("pkg");
    let roots = paths(&["[x]", "[x]/a b.ts", "../shared"]);
    let expected = ["../shared/café.ts", "[x]/a b.ts"];
    assert_eq!(files(&cwd, &roots), expected);
    let absolute: Vec<_> = roots.iter().map(|path| cwd.join(path)).collect();
    assert_eq!(files(&cwd, &absolute), expected);
    assert_eq!(files(&cwd, &[]), ["[x]/a b.ts", "x/a b.ts"]);
    write(root, "pkg/a.ts", "");
    write(root, "pkg/a/x.ts", "");
    assert_eq!(files(&cwd, &paths(&["a", "a.ts"])), ["a.ts", "a/x.ts"]);
}

#[cfg(unix)]
#[test]
fn git_records_preserve_filename_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    #[cfg(target_os = "linux")]
    let bytes = b"line\n\xff.ts".as_slice();
    #[cfg(not(target_os = "linux"))]
    let bytes = b"line\nend.ts".as_slice();
    let name = Path::new(OsStr::from_bytes(bytes));
    fs::write(root.join(name), "").unwrap();
    let result = discover(&[], &DiscoveryOptions::new(root)).unwrap();
    assert_eq!(result.files, [name]);
}

#[test]
fn propagate_git_failures_before_and_after_worktree_detection() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let opts = DiscoveryOptions::new(root);
    write(root, ".git", "broken gitfile");
    let error = discover(&[], &opts).unwrap_err();
    assert_eq!(error.path(), root.canonicalize().unwrap());
    assert!(error.to_string().contains(".git"), "{error}");

    fs::remove_file(root.join(".git")).unwrap();
    git(root, &["init", "-q"]);
    write(root, ".git/index", "broken index");
    let error = discover(&[], &opts).unwrap_err();
    assert!(error.to_string().contains("git ls-files"), "{error}");
}
