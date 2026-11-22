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

#[test]
fn match_path_globs_and_account_for_unused_exclusions() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    for name in [
        "src/.gen/a.ts",
        "src/A.ts",
        "pkg/src/.gen/b.ts",
        "src/tag.md",
        "dist/a.ts",
    ] {
        write(cwd, name, "");
    }
    let mut opts = DiscoveryOptions::new(cwd);
    opts.defaults = vec![flag("dist/**")];
    opts.excludes = [
        "src/*/[ab].{ts,js}",
        "**/.gen/a.ts",
        "src/?.ts",
        "src/tag.md",
        "dist/**",
        "absent/**",
    ]
    .map(flag)
    .to_vec();
    opts.excludes[3].origin = GlobOrigin::Config(PathBuf::from("report.toml"));
    opts.excludes.push(ExcludePattern {
        pattern: "absent/**".to_owned(),
        origin: GlobOrigin::Default,
    });
    for use_git in [false, true] {
        if use_git {
            git(cwd, &["init", "-q"]);
        }
        let scan = discover(&[], &opts).unwrap();
        assert_eq!(scan.files, paths(&["pkg/src/.gen/b.ts"]));
        assert_eq!(scan.unused_excludes, opts.excludes[3..]);
    }
}

#[test]
fn path_globs_are_anchored_literal_and_case_sensitive() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    for file in [
        "!direct.ts",
        "direct.ts",
        "a[1].ts",
        "a1.ts",
        "src/B.ts",
        "src/a.ts",
        "pkg/src/a.ts",
        "src/nested/a.ts",
    ] {
        write(cwd, file, "");
    }
    let mut opts = DiscoveryOptions::new(cwd);
    opts.excludes = [
        "!direct.ts",
        r"a\[1\].ts",
        "src/[a].ts",
        "src/?.ts",
        "src/*.ts",
        "src/*.js",
    ]
    .map(flag)
    .to_vec();
    let scan = discover(&[], &opts).unwrap();
    assert_eq!(
        scan.files,
        paths(&["a1.ts", "direct.ts", "pkg/src/a.ts", "src/nested/a.ts"])
    );
    assert_eq!(scan.unused_excludes, [flag("src/*.js")]);
    opts.excludes = vec![flag("src/b.ts")];
    assert!(
        discover(&[], &opts)
            .unwrap()
            .files
            .contains(&PathBuf::from("src/B.ts"))
    );
    opts.excludes = vec![flag("src?a.ts")];
    let question = discover(&[], &opts).unwrap();
    assert!(question.files.contains(&PathBuf::from("src/a.ts")));
    assert_eq!(question.unused_excludes, [flag("src?a.ts")]);
}

#[test]
fn apply_discovery_scope_to_current_and_old_git_paths() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    for file in [
        "src/live.ts",
        "src/cache/current.ts",
        "src/tracked.ts",
        "src/vendor/v.ts",
        "src/sub/v.ts",
        "src/gen/a.ts",
        "other/live.ts",
    ] {
        write(root, file, "");
    }
    git(root, &["add", "src/tracked.ts"]);
    fs::remove_file(root.join("src/tracked.ts")).unwrap();
    write(root, ".gitignore", "src/cache/**\nsrc/tracked.ts\n");
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
    let mut opts = DiscoveryOptions::new(root);
    opts.excludes.push(flag("src/gen/**"));
    let result = discover(&paths(&["src"]), &opts).unwrap();
    for (path, kept) in [
        ("src/live.ts", true),
        ("src/old.ts", true),
        ("src/tracked.ts", true),
        ("src/cache/current.ts", false),
        ("src/cache/old.ts", false),
        ("src/vendor/old.ts", false),
        ("src/sub/old.ts", false),
        ("src/gen/old.ts", false),
        ("src/gone.test.ts", false),
        ("src/old.d.ts", false),
        ("other/old.ts", false),
        ("src2/old.ts", false),
        ("other/live.ts", false),
        ("src/live.md", false),
    ] {
        assert_eq!(result.scope.keeps(Path::new(path)).unwrap(), kept, "{path}");
    }
    assert!(result.scope.keeps(&root.join("src/old.ts")).unwrap());
    let file = discover(&paths(&["src/live.ts"]), &opts).unwrap();
    assert!(file.scope.keeps(Path::new("src/live.ts")).unwrap());
    assert!(!file.scope.keeps(Path::new("src/old.ts")).unwrap());
    assert!(!file.scope.keeps(Path::new("src/live.ts/child.ts")).unwrap());
}

#[test]
fn filesystem_scope_keeps_historical_siblings_and_prunes_walk_boundaries() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "pkg/main.ts", "");
    write(root, "shared/a.ts", "");
    let mut opts = DiscoveryOptions::new(root.join("pkg"));
    opts.defaults.clear();
    let result = discover(&paths(&[".", "../shared"]), &opts).unwrap();
    for (path, kept) in [
        ("../shared/gone.ts", true),
        ("./old.ts", true),
        ("../other/old.ts", false),
        ("node_modules/old.ts", false),
        ("../shared/.git/old.ts", false),
        ("../shared/x.d.mts", false),
    ] {
        assert_eq!(result.scope.keeps(Path::new(path)).unwrap(), kept, "{path}");
    }
    write(root, "pkg/born-after.ts", "");
    assert!(!result.scope.keeps(Path::new("born-after.ts")).unwrap());
}

#[test]
fn an_ignore_check_failure_retains_the_candidate_and_command() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "src/a.ts", "");
    let scan = discover(&paths(&["src"]), &DiscoveryOptions::new(root)).unwrap();
    write(root, ".git/index", "broken index");
    let error = scan.scope.keeps(Path::new("src/old.ts")).unwrap_err();
    assert_eq!(error.path(), Path::new("src/old.ts"));
    assert!(error.to_string().contains("git check-ignore"), "{error}");
}

#[cfg(any(unix, windows))]
#[test]
fn absent_paths_beneath_a_directory_link_are_outside_scope() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "real/live.ts", "");
    if !link(&root.join("real"), &root.join("alias"), true) {
        return;
    }
    for use_git in [false, true] {
        if use_git {
            git(root, &["init", "-q"]);
        }
        let scan = discover(&[], &DiscoveryOptions::new(root)).unwrap();
        assert!(!scan.scope.keeps(Path::new("alias/old.ts")).unwrap());
        assert!(scan.scope.keeps(Path::new("real/old.ts")).unwrap());
        let link_scan = discover(&paths(&["alias"]), &DiscoveryOptions::new(root)).unwrap();
        assert!(!link_scan.scope.keeps(Path::new("alias/old.ts")).unwrap());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn an_explicit_walk_root_below_a_link_keeps_former_children() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    write(cwd, "real/sub/live.ts", "");
    if !link(&cwd.join("real"), &cwd.join("alias"), true) {
        return;
    }
    let scan = discover(&paths(&["alias/sub"]), &DiscoveryOptions::new(cwd)).unwrap();
    assert_eq!(scan.files, paths(&["alias/sub/live.ts"]));
    assert!(scan.scope.keeps(Path::new("alias/sub/old.ts")).unwrap());
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
    assert_eq!(error.path(), Path::new(""));
    assert!(
        error.to_string().starts_with("invalid glob bad/["),
        "{error}"
    );
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
        let expected = if use_git { vec![] } else { vec!["alias/a.ts"] };
        assert_eq!(files(cwd, &paths(&["alias/a.ts"])), expected);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn retain_a_cached_file_reached_through_a_replaced_parent() {
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
    assert_eq!(files(root, &paths(&["src"])), ["src/old/x.ts"]);
    write(root, "src/loop/x.ts", "");
    git(root, &["add", "src/loop/x.ts"]);
    fs::remove_file(root.join("src/loop/x.ts")).unwrap();
    fs::remove_dir(root.join("src/loop")).unwrap();
    let cyclic = root.join("src/loop");
    assert!(link(&cyclic, &cyclic, true));
    let error = discover(&paths(&["src"]), &DiscoveryOptions::new(root)).unwrap_err();
    assert!(error.path().ends_with("src/loop/x.ts"), "{error}");
}

#[cfg(any(unix, windows))]
fn check_alternate_absolute_root(
    cwd: &Path,
    alias: &Path,
    expected: &Path,
) {
    write(cwd, "src/a.ts", "");
    let root = alias.join("src/a.ts");
    let roots = [root.clone()];
    let opts = DiscoveryOptions::new(cwd);
    assert_eq!(discover(&roots, &opts).unwrap().files, [expected]);
    git(cwd, &["init", "-q"]);
    let error = discover(&roots, &opts).unwrap_err();
    assert_eq!(error.path(), root);
    assert!(
        error.to_string().contains("outside the repository"),
        "{error}"
    );
    assert!(
        error.to_string().contains(&cwd.display().to_string()),
        "{error}"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn an_absolute_alias_above_the_scan_boundary_keeps_its_spelling() {
    let dir = tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    fs::create_dir_all(base.join("physical/repo")).unwrap();
    if !link(&base.join("physical"), &base.join("alias"), true) {
        return;
    }
    check_alternate_absolute_root(
        &base.join("physical/repo"),
        &base.join("alias/repo"),
        Path::new("../../alias/repo/src/a.ts"),
    );
    let walk_dir = tempdir().unwrap();
    let walk_base = walk_dir.path().canonicalize().unwrap();
    fs::create_dir_all(walk_base.join("physical/repo")).unwrap();
    assert!(link(
        &walk_base.join("physical"),
        &walk_base.join("alias"),
        true
    ));
    write(&walk_base.join("physical/repo"), "src/a.ts", "");
    assert_eq!(
        files(
            &walk_base.join("alias/repo"),
            &[walk_base.join("physical/repo/src/a.ts")]
        ),
        ["../../physical/repo/src/a.ts"]
    );
}

#[cfg(any(unix, windows))]
#[test]
fn a_git_invocation_alias_accepts_both_spellings_of_its_physical_roots() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    git(&root, &["init", "-q"]);
    for name in [
        "packages/pkg/mine.ts",
        "packages/shared/physical.ts",
        "shared/logical.ts",
    ] {
        write(&root, name, "");
    }
    let alias = root.join("invoked");
    if !link(&root.join("packages/pkg"), &alias, true) {
        return;
    }
    for roots in [
        vec![alias.join("mine.ts"), PathBuf::from("../shared")],
        vec![
            root.join("packages/pkg/mine.ts"),
            root.join("packages/shared"),
        ],
    ] {
        assert_eq!(files(&alias, &roots), ["../shared/physical.ts", "mine.ts"]);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_tmp_and_private_tmp_preserve_the_invocation_spelling() {
    let dir = tempfile::tempdir_in("/private/tmp").unwrap();
    let name = dir.path().file_name().unwrap();
    let alias = Path::new("/tmp").join(name);
    let expected = Path::new("../../../tmp").join(name).join("src/a.ts");
    check_alternate_absolute_root(dir.path(), &alias, &expected);
}

#[test]
fn a_recursive_exclusion_cannot_consume_parent_directory_components() {
    let dir = tempdir().unwrap();
    let cwd = dir.path().join("pkg");
    for file in [
        "pkg/in.ts",
        "shared/a.test.ts",
        "shared/dist/b.ts",
        "shared/gen/g.ts",
        "shared/ok.ts",
    ] {
        write(dir.path(), file, "");
    }
    let roots = paths(&[".", "../shared"]);
    for use_git in [false, true] {
        if use_git {
            git(dir.path(), &["init", "-q"]);
        }
        let mut options = DiscoveryOptions::new(&cwd);
        options.excludes = ["**/gen/**", "**/ok.ts"].map(flag).to_vec();
        let scan = discover(&roots, &options).unwrap();
        assert_eq!(
            scan.files,
            paths(&[
                "../shared/a.test.ts",
                "../shared/dist/b.ts",
                "../shared/gen/g.ts",
                "../shared/ok.ts",
                "in.ts"
            ])
        );
        assert_eq!(scan.unused_excludes, options.excludes);
        assert!(
            scan.scope
                .keeps(Path::new("../shared/old.test.ts"))
                .unwrap()
        );
        options.excludes = ["../shared/gen/**", "../**/ok.ts"].map(flag).to_vec();
        let scan = discover(&roots, &options).unwrap();
        assert_eq!(
            scan.files,
            paths(&["../shared/a.test.ts", "../shared/dist/b.ts", "in.ts"])
        );
        assert_eq!(scan.unused_excludes, Vec::<ExcludePattern>::new());
        assert!(!scan.scope.keeps(Path::new("../shared/gen/old.ts")).unwrap());
    }
}

fn check_missing_root_failures() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    write(cwd, "file.ts", "");
    write(cwd, "deleted.ts", "");
    for use_git in [false, true] {
        if use_git {
            git(cwd, &["init", "-q"]);
            git(cwd, &["add", "deleted.ts"]);
            fs::remove_file(cwd.join("deleted.ts")).unwrap();
        }
        for missing in ["gone.ts", "file.ts/child.ts", "node_modules/gone.ts"] {
            let error = discover(&paths(&[".", missing]), &DiscoveryOptions::new(cwd)).unwrap_err();
            assert_eq!(error.path(), Path::new(missing), "git={use_git}: {error}");
            assert_eq!(error.side(), None);
            assert!(error.to_string().contains(missing), "{error}");
            assert!(error.to_string().contains("path does not exist"), "{error}");
        }
        if use_git {
            let error = discover(&paths(&["deleted.ts"]), &DiscoveryOptions::new(cwd)).unwrap_err();
            assert_eq!(error.path(), Path::new("deleted.ts"));
            assert!(error.to_string().contains("path does not exist"), "{error}");
        }
        let nul_root = Path::new("invalid\0path");
        let nul_error = discover(&[nul_root.to_owned()], &DiscoveryOptions::new(cwd)).unwrap_err();
        assert_eq!(nul_error.path(), nul_root);
        assert!(
            !nul_error.to_string().contains("path does not exist"),
            "{nul_error}"
        );
    }
    let absent_cwd = cwd.join("no-such-dir");
    let cwd_error = discover(&[], &DiscoveryOptions::new(&absent_cwd)).unwrap_err();
    assert_eq!(cwd_error.path(), absent_cwd.as_path());
    assert!(cwd_error.to_string().contains("no-such-dir"), "{cwd_error}");
}

fn check_outside_worktree_failures() {
    let dir = tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let app = repo.join("app");
    write(&repo, "app/a.ts", "");
    write(&base, "other/b.ts", "");
    write(&base, "repo-other/c.ts", "");
    git(&repo, &["init", "-q"]);
    let repo_text = repo.display().to_string();
    for (cwd, arg) in [
        (&repo, PathBuf::from("../other")),
        (&repo, PathBuf::from("../absent-other")),
        (&app, PathBuf::from("../..")),
        (&app, PathBuf::from("../../repo-other/c.ts")),
        (&app, base.join("repo-other/c.ts")),
    ] {
        let error = discover(std::slice::from_ref(&arg), &DiscoveryOptions::new(cwd)).unwrap_err();
        assert_eq!(error.path(), arg.as_path());
        assert_eq!(error.side(), None);
        let text = error.to_string();
        assert!(text.contains(&arg.display().to_string()), "{text}");
        assert!(text.contains(&repo_text), "{text}");
        assert!(text.contains("outside the repository"), "{text}");
    }
}

fn check_malformed_glob_failures() {
    let dir = tempdir().unwrap();
    let cwd = dir.path();
    let cwd_text = cwd.canonicalize().unwrap().display().to_string();
    for use_git in [false, true] {
        if use_git {
            git(cwd, &["init", "-q"]);
        }
        let mut opts = DiscoveryOptions::new(cwd);
        opts.excludes = vec![flag("src/[")];
        let error = discover(&[], &opts).unwrap_err();
        assert_eq!(error.path(), Path::new(""));
        assert_eq!(error.side(), None);
        let text = error.to_string();
        assert!(text.starts_with("invalid glob src/["), "{text}");
        assert!(!text.contains(&cwd_text), "{text}");

        let before_missing = discover(&paths(&["missing-dir"]), &opts).unwrap_err();
        assert!(
            before_missing.to_string().starts_with("invalid glob src/["),
            "{before_missing}"
        );

        let config_path = PathBuf::from("crapts.toml");
        opts.excludes = vec![ExcludePattern {
            pattern: "src/[".to_owned(),
            origin: GlobOrigin::Config(config_path.clone()),
        }];
        let config_error = discover(&[], &opts).unwrap_err();
        assert_eq!(config_error.path(), config_path.as_path());
        assert!(
            config_error
                .to_string()
                .starts_with("crapts.toml: invalid glob src/["),
            "{config_error}"
        );
    }
}

fn check_corrupt_index_failures() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "src/a.ts", "");
    write(root, ".git/index", "corrupt index bytes");
    for roots in [Vec::new(), paths(&["src"])] {
        let error = discover(&roots, &DiscoveryOptions::new(root)).unwrap_err();
        assert_eq!(error.side(), None);
        assert!(error.to_string().contains("git ls-files"), "{error}");
    }
}

#[test]
fn report_discovery_failures() {
    check_missing_root_failures();
    check_outside_worktree_failures();
    check_malformed_glob_failures();
    check_corrupt_index_failures();
}

#[cfg(any(unix, windows))]
#[test]
fn explicit_broken_and_cyclic_symlink_roots_follow_metadata_rules() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    if !link(&root.join("missing.ts"), &root.join("dangling.ts"), false) {
        return;
    }
    let cyclic = root.join("loop");
    assert!(link(&cyclic, &cyclic, true));
    for use_git in [false, true] {
        if use_git {
            git(root, &["init", "-q"]);
        }
        assert_eq!(files(root, &paths(&["dangling.ts"])), Vec::<String>::new());
        let error = discover(&paths(&["loop/x.ts"]), &DiscoveryOptions::new(root)).unwrap_err();
        assert_eq!(error.path(), Path::new("loop/x.ts"));
        assert!(
            !error.to_string().contains("path does not exist"),
            "{error}"
        );
    }
}
