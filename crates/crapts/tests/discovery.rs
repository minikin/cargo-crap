use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crapts::{DiscoveryOptions, ExcludePattern, GlobOrigin, Mode, Side, analyze, discover};
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
}

#[test]
fn retain_outer_index_files_beneath_a_nested_git_entry() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "src/vendor/old.ts", "");
    write(root, "src/bad/old.ts", "");
    git(root, &["add", "src/vendor/old.ts", "src/bad/old.ts"]);
    git(&root.join("src/vendor"), &["init", "-q"]);
    write(root, "src/bad/.git", "bad marker");
    write(root, ".gitignore", ".git\n");
    assert_eq!(
        files(root, &paths(&["src"])),
        ["src/bad/old.ts", "src/vendor/old.ts"]
    );
    assert_eq!(
        files(root, &paths(&["src/vendor/old.ts"])),
        ["src/vendor/old.ts"]
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
    fs::write(root.join(name), "const kept = (x: number) => x").unwrap();
    let result = discover(&[], &DiscoveryOptions::new(root)).unwrap();
    assert_eq!(result.files, [name]);
    let row = analyze(
        name,
        &fs::read(root.join(name)).unwrap(),
        Mode::Strict(Side::Head),
    )
    .unwrap();
    assert_eq!(row.rows[0].name, "kept");
}

#[test]
fn analyze_suffix_only_filenames_as_discovered() {
    let dir = tempdir().unwrap();
    for (name, source) in [
        (".ts", "const f = (x: number) => x"),
        (".js", "const f = () => <b/>"),
    ] {
        write(dir.path(), name, source);
    }
    let paths = discover(&[], &DiscoveryOptions::new(dir.path()))
        .unwrap()
        .files;
    assert_eq!(paths, [Path::new(".js"), Path::new(".ts")]);
    for path in paths {
        let source = fs::read(dir.path().join(&path)).unwrap();
        let file = analyze(&path, &source, Mode::Strict(Side::Head)).unwrap();
        assert_eq!(file.rows[0].name, "f");
        assert_eq!(file.rows[0].file, path);
    }
}

#[cfg(unix)]
#[test]
fn an_ascii_source_suffix_identifies_a_non_utf8_filename() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"\xff.cts"));
    let file = analyze(path, b"function f(x: number) {}", Mode::Strict(Side::Head)).unwrap();
    assert_eq!(file.rows[0].file, path);
    assert_eq!(file.rows[0].name, "f");
}

#[test]
fn walk_when_a_gitfile_does_not_identify_a_worktree() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, ".git", "broken gitfile");
    write(root, "real/a.ts", "");
    assert_eq!(files(root, &[]), ["real/a.ts"]);
}

#[test]
fn sort_unicode_paths_in_utf16_order() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "\u{10000}.ts", "");
    write(root, "\u{e000}.ts", "");
    assert_eq!(files(root, &[]), ["\u{10000}.ts", "\u{e000}.ts"]);
    git(root, &["init", "-q"]);
    assert_eq!(files(root, &[]), ["\u{10000}.ts", "\u{e000}.ts"]);
}

#[test]
fn a_hook_environment_cannot_redirect_the_discovery_index() {
    let key = "CRAPTS_TEST_HOOK_CWD";
    if let Some(cwd) = std::env::var_os(key) {
        assert_eq!(files(Path::new(&cwd), &[]), ["mine.ts"]);
        return;
    }
    let ours = tempdir().unwrap();
    let theirs = tempdir().unwrap();
    for (root, name) in [(ours.path(), "mine.ts"), (theirs.path(), "other.ts")] {
        git(root, &["init", "-q"]);
        write(root, name, "");
        git(root, &["add", name]);
        write(root, ".gitignore", "*\n");
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_hook_environment_cannot_redirect_the_discovery_index",
            "--nocapture",
        ])
        .env(key, ours.path())
        .env("GIT_DIR", theirs.path().join(".git"))
        .env("GIT_WORK_TREE", theirs.path())
        .env("GIT_INDEX_FILE", theirs.path().join(".git/index"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

fn walking_fixture(root: &Path) {
    for name in ["real/a.ts", ".git/info/a.ts", "deps/node_modules/a.ts"] {
        write(root, name, "");
    }
}

fn without_exclusions(
    cwd: &Path,
    roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut options = DiscoveryOptions::new(cwd);
    options.defaults.clear();
    discover(roots, &options).unwrap().files
}

fn flag(pattern: &str) -> ExcludePattern {
    ExcludePattern {
        pattern: pattern.to_owned(),
        origin: GlobOrigin::Flag,
    }
}

#[test]
fn select_the_eight_source_extensions_case_sensitively() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    for suffix in [
        "ts", "mts", "cts", "tsx", "js", "mjs", "cjs", "jsx", "json", "d.ts", "d.mts", "d.cts",
        "d.ts.js",
    ] {
        write(root, &format!("a.{suffix}"), "");
    }
    write(root, "b.TS", "");
    let expected = [
        "a.cjs",
        "a.cts",
        "a.d.ts.js",
        "a.js",
        "a.jsx",
        "a.mjs",
        "a.mts",
        "a.ts",
        "a.tsx",
    ];
    assert_eq!(files(root, &[]), expected);
    git(root, &["init", "-q"]);
    assert_eq!(files(root, &[]), expected);
}

#[test]
fn apply_default_and_caller_supplied_exclusions() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    git(cwd, &["init", "-q"]);
    let defaults = [
        "node_modules/p/a.js",
        "pkg/dist/a.ts",
        "build/b.ts",
        "coverage/a.js",
        "lib/a.test.ts",
        "a.spec.mts",
        "src/.hidden/test.stories.tsx",
        "src/a.min.js",
        "__tests__/a.cts",
        "src/__mocks__/a.jsx",
    ];
    for name in defaults.into_iter().chain(["src/main.ts", "src/gen/a.ts"]) {
        write(cwd, name, "");
        git(cwd, &["add", "-f", "--", name]);
    }
    let mut options = DiscoveryOptions::new(cwd);
    options.excludes.push(flag("src/gen/**"));
    assert_eq!(
        discover(&[], &options).unwrap().files,
        paths(&["src/main.ts"])
    );
    options.defaults.clear();
    let mut expected = paths(&defaults);
    expected.push(PathBuf::from("src/main.ts"));
    expected.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    assert_eq!(discover(&[], &options).unwrap().files, expected);
    options.defaults.push(flag("src/main.ts"));
    assert_eq!(
        discover(&paths(&["src"]), &options).unwrap().files,
        paths(&[
            "src/.hidden/test.stories.tsx",
            "src/__mocks__/a.jsx",
            "src/a.min.js"
        ])
    );
}

#[test]
fn explicit_file_roots_use_the_same_filters_as_directory_roots() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    write(cwd, "only.json", "");
    write(cwd, "one.spec.ts", "");
    write(cwd, "valid.ts", "");
    let roots = paths(&["only.json", "one.spec.ts", "valid.ts"]);
    let mut options = DiscoveryOptions::new(cwd);
    options.excludes.push(flag("valid.ts"));
    assert_eq!(
        discover(&roots, &options).unwrap().files,
        Vec::<PathBuf>::new()
    );
    git(cwd, &["init", "-q"]);
    assert_eq!(
        discover(&roots, &options).unwrap().files,
        Vec::<PathBuf>::new()
    );
}

#[test]
fn an_invalid_replacement_default_names_the_pattern() {
    let dir = tempdir().unwrap();
    write(dir.path(), "one.ts", "");
    let mut options = DiscoveryOptions::new(dir.path());
    options.defaults = vec![flag("bad/[")];
    let error = discover(&paths(&["one.ts"]), &options).unwrap_err();
    assert!(error.to_string().contains("bad/["), "{error}");
}

#[test]
fn walk_regular_files_outside_a_git_worktree() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    walking_fixture(cwd);
    assert_eq!(
        without_exclusions(cwd, &paths(&["."])),
        paths(&["real/a.ts"])
    );
    assert_eq!(
        without_exclusions(
            cwd,
            &paths(&["real/a.ts", "real", "deps/node_modules/a.ts", ".git/info"])
        ),
        paths(&["real/a.ts"])
    );
    write(cwd, "shared/b.ts", "");
    assert_eq!(
        files(&cwd.join("real"), &paths(&[".", "../shared"])),
        ["../shared/b.ts", "a.ts"]
    );
    assert_eq!(files(cwd, &[cwd.join("real/a.ts")]), ["real/a.ts"]);
}

#[test]
fn a_bare_repository_does_not_supply_a_worktree() {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init", "--bare", "-q"]);
    write(dir.path(), "outside.ts", "");
    assert_eq!(files(dir.path(), &paths(&["outside.ts"])), ["outside.ts"]);
}

#[test]
fn walk_with_git_removed_from_path() {
    let key = "CRAPTS_TEST_WALK_WITHOUT_GIT";
    if let Some(directory) = std::env::var_os(key) {
        assert_eq!(
            without_exclusions(Path::new(&directory), &[]),
            paths(&["real/a.ts"])
        );
        return;
    }
    let dir = tempdir().unwrap();
    walking_fixture(dir.path());
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "walk_with_git_removed_from_path", "--nocapture"])
        .env(key, dir.path())
        .env("PATH", dir.path().join("no-git"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[cfg(unix)]
#[test]
fn an_unlaunchable_git_is_an_input_error() {
    use std::os::unix::fs::PermissionsExt;
    let key = "CRAPTS_TEST_UNLAUNCHABLE_GIT";
    if let Some(cwd) = std::env::var_os(key) {
        let error = discover(&[], &DiscoveryOptions::new(cwd)).unwrap_err();
        assert!(error.to_string().contains("git rev-parse"), "{error}");
        return;
    }
    let dir = tempdir().unwrap();
    write(dir.path(), "git", "");
    fs::set_permissions(dir.path().join("git"), fs::Permissions::from_mode(0o644)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "an_unlaunchable_git_is_an_input_error",
            "--nocapture",
        ])
        .env(key, dir.path())
        .env("PATH", dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[cfg(any(unix, windows))]
fn link(
    target: &Path,
    path: &Path,
    directory: bool,
) -> bool {
    #[cfg(unix)]
    let result = {
        let _ = directory;
        std::os::unix::fs::symlink(target, path)
    };
    #[cfg(windows)]
    let result = if directory {
        std::os::windows::fs::symlink_dir(target, path)
    } else {
        std::os::windows::fs::symlink_file(target, path)
    };
    #[cfg(windows)]
    if result
        .as_ref()
        .is_err_and(|e| e.raw_os_error() == Some(1314))
    {
        return false;
    }
    result.unwrap();
    true
}

#[cfg(any(unix, windows))]
#[test]
fn skip_symbolic_links_to_files_and_directories() {
    for use_git in [false, true] {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        write(cwd, "real/a.ts", "");
        if !link(&cwd.join("real/a.ts"), &cwd.join("link.ts"), false) {
            return;
        }
        assert!(link(&cwd.join("real"), &cwd.join("alias"), true));
        if use_git {
            git(cwd, &["init", "-q"]);
        }
        assert_eq!(files(cwd, &[]), ["real/a.ts"]);
        assert_eq!(
            files(cwd, &paths(&["link.ts", "alias"])),
            Vec::<String>::new()
        );
        assert_eq!(files(cwd, &paths(&["alias/a.ts"])), Vec::<String>::new());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn skip_cached_children_when_their_parent_becomes_a_symlink() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "src/old/x.ts", "");
    git(root, &["add", "src/old/x.ts"]);
    fs::remove_file(root.join("src/old/x.ts")).unwrap();
    fs::remove_dir(root.join("src/old")).unwrap();
    write(root, "target/x.ts", "");
    if !link(&root.join("target"), &root.join("src/old"), true) {
        return;
    }
    write(root, "src/loop/x.ts", "");
    git(root, &["add", "src/loop/x.ts"]);
    fs::remove_file(root.join("src/loop/x.ts")).unwrap();
    fs::remove_dir(root.join("src/loop")).unwrap();
    let cyclic = root.join("src/loop");
    assert!(link(&cyclic, &cyclic, true));
    assert_eq!(files(root, &paths(&["src"])), Vec::<String>::new());
}
