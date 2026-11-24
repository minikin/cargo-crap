use cargo_crap_core::git::{Change, ChangeStatus, CommitId, GitError, Hunk, Mode, Repo, Side};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const ISOLATED: &str = "CARGO_CRAP_GIT_TEST_ISOLATED";

const REPOSITORY_OVERRIDES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

macro_rules! test_name {
    () => {{
        fn here() {}
        let path = std::any::type_name_of_val(&here);
        let path = path.strip_suffix("::here").expect("a function path");
        path.rsplit("::").next().expect("a test name")
    }};
}

/// The reader's own git calls inherit this process's environment, so each
/// test runs again in a child whose Git reads no user or system config: a
/// global excludes file or diff setting cannot change what it sees.
macro_rules! isolate {
    () => {
        if !isolated(test_name!(), &[]) {
            return;
        }
    };
}

fn isolated(
    test: &str,
    env: &[(&str, OsString)],
) -> bool {
    if std::env::var_os(ISOLATED).is_some() {
        return true;
    }
    let config = TempDir::new().expect("temp dir");
    let empty = config.path().join("empty.gitconfig");
    fs::write(&empty, "").expect("empty config");
    let mut child = Command::new(std::env::current_exe().expect("test binary"));
    child
        .args([test, "--exact", "--test-threads=1"])
        .env(ISOLATED, "1")
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for name in REPOSITORY_OVERRIDES {
        child.env_remove(name);
    }
    for (name, value) in env {
        child.env(name, value);
    }
    let output = child.output().expect("re-running the test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"), "{test}: {stdout}");
    assert!(
        output.status.success(),
        "{test}: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

/// A scratch repository whose own git calls read no user or system config,
/// so a signing key or a diff setting on the machine cannot change a case.
struct Fixture {
    dir: TempDir,
    empty_config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let empty_config = dir.path().join("empty.gitconfig");
        fs::write(&empty_config, "").expect("empty config");
        let fixture = Fixture { dir, empty_config };
        fs::create_dir(fixture.root()).expect("repo dir");
        fixture
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn git_in(
        &self,
        dir: &Path,
        args: &[&str],
    ) -> String {
        let output = self.command_in(dir, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8 git output")
    }

    fn git(
        &self,
        args: &[&str],
    ) -> String {
        self.git_in(&self.root(), args)
    }

    fn command_in(
        &self,
        dir: &Path,
        args: &[&str],
    ) -> std::process::Output {
        let mut command = Command::new("git");
        for name in REPOSITORY_OVERRIDES {
            command.env_remove(name);
        }
        command
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", &self.empty_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap_or_else(|e| panic!("running git {args:?}: {e}"))
    }

    fn git_ok(
        &self,
        args: &[&str],
    ) -> bool {
        self.command_in(&self.root(), args).status.success()
    }

    fn git_bytes(
        &self,
        args: &[&str],
    ) -> Vec<u8> {
        let output = self.command_in(&self.root(), args);
        assert!(output.status.success(), "git {args:?} failed");
        output.stdout
    }

    fn write(
        &self,
        path: &str,
        contents: &str,
    ) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        fs::write(path, contents).expect("write file");
    }

    fn commit(
        &self,
        message: &str,
    ) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
    }

    fn rev(
        &self,
        rev: &str,
    ) -> String {
        self.git(&["rev-parse", rev]).trim().to_owned()
    }

    fn standard() -> Self {
        let fixture = Fixture::new();
        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.write(
            "src/app.ts",
            "export function hot(x: number) {\n  return 0\n}\n",
        );
        fixture.commit("B");
        fixture.git(&["checkout", "-q", "-b", "feat"]);
        fixture
    }
}

fn file_url(path: &Path) -> String {
    let path = path.to_str().expect("utf-8 temp path").replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn same_path(
    a: &Path,
    b: &Path,
) -> bool {
    fs::canonicalize(a).expect("canonical a") == fs::canonicalize(b).expect("canonical b")
}

#[test]
fn the_reader_resolves_the_revision_merge_base_and_head_of_a_worktree() {
    isolate!();
    let fixture = Fixture::standard();
    let b = fixture.rev("HEAD");
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("src/other.ts", "export const t = 1\n");
    fixture.commit("T");
    fixture.git(&["checkout", "-q", "feat"]);
    fixture.write(
        "src/app.ts",
        "export function hot(x: number) {\n  return 1\n}\n",
    );

    let repo = Repo::open(&fixture.root().join("src")).expect("open");
    let head = repo.head().expect("head");
    let main = repo.resolve_commit("main").expect("resolve main");

    assert!(same_path(repo.root(), &fixture.root()));
    assert!(repo.root().is_absolute());
    assert_eq!(main.as_str(), fixture.rev("main"));

    let base = repo.merge_base(&main, &head).expect("merge base");
    assert_eq!(base.commit.as_str(), b);
    assert_eq!(base.count, 1);
    assert!(!base.shallow);

    assert_eq!(head.as_str(), fixture.rev("HEAD"));
    assert!(repo.is_dirty().expect("dirty"));

    fixture.git(&["checkout", "--", "src/app.ts"]);
    fixture.write(".git/info/exclude", "ignored.txt\n");
    fixture.write("ignored.txt", "not a change\n");
    assert!(!repo.is_dirty().expect("clean"));
    fixture.write("untracked.txt", "a change\n");
    assert!(repo.is_dirty().expect("untracked is dirty"));
}

#[test]
fn criss_cross_history_uses_gits_merge_base_and_records_the_count() {
    isolate!();
    let fixture = Fixture::standard();
    fixture.write("feat.txt", "f1\n");
    fixture.commit("F1");
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("main.txt", "m1\n");
    fixture.commit("M1");
    let m1 = fixture.rev("HEAD");
    fixture.git(&["merge", "-q", "--no-ff", "-m", "M2", "feat"]);
    fixture.git(&["checkout", "-q", "feat"]);
    fixture.git(&["merge", "-q", "--no-ff", "-m", "F2", &m1]);
    let all = fixture.git(&["merge-base", "--all", "main", "HEAD"]);
    assert_eq!(all.lines().count(), 2, "the fixture is criss-cross: {all}");

    let repo = Repo::open(&fixture.root()).expect("open");
    let main = repo.resolve_commit("main").expect("main");
    let head = repo.head().expect("head");
    let base = repo.merge_base(&main, &head).expect("merge base");

    assert_eq!(
        base.commit.as_str(),
        fixture.git(&["merge-base", "main", "HEAD"]).trim()
    );
    assert_eq!(base.count, 2);
    assert!(!base.shallow);
}

#[test]
fn a_merge_base_found_in_a_shallow_clone_is_used_and_marked_shallow() {
    isolate!();
    let origin = Fixture::new();
    origin.git(&["init", "-q", "-b", "main"]);
    for name in ["A0", "A1"] {
        origin.write("history.txt", name);
        origin.commit(name);
    }
    origin.write("src/app.ts", "B\n");
    origin.commit("B");
    let b = origin.rev("HEAD");
    origin.git(&["checkout", "-q", "-b", "feat"]);
    origin.write("feat.txt", "F1\n");
    origin.commit("F1");
    origin.git(&["checkout", "-q", "main"]);
    origin.write("main.txt", "T\n");
    origin.commit("T");
    origin.git(&["checkout", "-q", "feat"]);
    let url = file_url(&origin.root());
    let clone = origin.dir.path().join("clone");
    origin.git_in(
        origin.dir.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "3",
            "--no-single-branch",
            &url,
            clone.to_str().expect("utf-8 temp path"),
        ],
    );

    let repo = Repo::open(&clone).expect("open clone");
    let main = repo.resolve_commit("origin/main").expect("origin/main");
    let head = repo.head().expect("head");
    let base = repo.merge_base(&main, &head).expect("merge base");

    assert_eq!(base.commit.as_str(), b);
    assert_eq!(base.count, 1);
    assert!(base.shallow);
}

#[test]
fn a_shallow_clone_without_a_shared_fetched_commit_is_a_shallow_history_error() {
    isolate!();
    let origin = Fixture::standard();
    origin.write("feat.txt", "F1\n");
    origin.commit("F1");
    origin.git(&["checkout", "-q", "main"]);
    origin.write("main.txt", "T\n");
    origin.commit("T");
    origin.git(&["checkout", "-q", "feat"]);
    let url = file_url(&origin.root());
    let clone = origin.dir.path().join("clone");
    origin.git_in(
        origin.dir.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            "--no-single-branch",
            &url,
            clone.to_str().expect("utf-8 temp path"),
        ],
    );

    let repo = Repo::open(&clone).expect("open clone");
    let main = repo.resolve_commit("origin/main").expect("origin/main");
    let head = repo.head().expect("head");
    let err = repo
        .merge_base(&main, &head)
        .expect_err("no fetched merge base");

    assert!(matches!(err, GitError::ShallowHistory { .. }), "{err:?}");
    let message = err.to_string();
    assert!(message.contains("shallow clone"), "{message}");
    assert!(message.contains("fetch-depth: 0"), "{message}");
    assert!(message.contains("git fetch --deepen"), "{message}");
}

#[test]
fn unrelated_histories_have_no_merge_base() {
    isolate!();
    let fixture = Fixture::standard();
    fixture.git(&["checkout", "-q", "--orphan", "other"]);
    fixture.write("other.txt", "unrelated\n");
    fixture.commit("O");
    fixture.git(&["checkout", "-q", "feat"]);

    let repo = Repo::open(&fixture.root()).expect("open");
    let other = repo.resolve_commit("other").expect("other");
    let head = repo.head().expect("head");
    let err = repo.merge_base(&other, &head).expect_err("unrelated");

    assert!(
        matches!(err, GitError::UnrelatedHistories { .. }),
        "{err:?}"
    );
    let message = err.to_string();
    assert!(message.contains("no merge base"), "{message}");
    assert!(message.contains("unrelated histories"), "{message}");
    assert!(message.contains(other.as_str()), "{message}");
    assert!(message.contains(head.as_str()), "{message}");
}

#[test]
fn a_directory_outside_any_worktree_is_not_a_worktree() {
    isolate!();
    let fixture = Fixture::new();
    let err = Repo::open(&fixture.root()).expect_err("no worktree");
    assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
    assert!(
        err.to_string().contains("not inside a Git worktree"),
        "{err}"
    );
}

#[test]
fn a_missing_directory_is_not_a_worktree() {
    isolate!();
    let fixture = Fixture::new();
    let err = Repo::open(&fixture.root().join("absent")).expect_err("missing dir");
    assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
}

#[test]
fn a_worktree_removed_after_it_was_opened_is_not_a_worktree() {
    isolate!();
    let fixture = Fixture::standard();
    let repo = Repo::open(&fixture.root()).expect("open");
    drop(fixture);
    let err = repo.head().expect_err("root is gone");
    assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
}

#[test]
fn ambient_git_variables_cannot_point_the_reader_at_another_repository() {
    if std::env::var_os(ISOLATED).is_none() {
        let other = Fixture::standard();
        other.commit("only in the other repository");
        other.write("foreign.txt", "staged only here\n");
        other.git(&["add", "foreign.txt"]);
        other.write("dirty.txt", "dirty\n");
        let git_dir = other.root().join(".git");
        let env = [
            ("GIT_DIR", git_dir.clone().into_os_string()),
            ("GIT_WORK_TREE", other.root().into_os_string()),
            ("GIT_INDEX_FILE", git_dir.join("index").into_os_string()),
            ("GIT_COMMON_DIR", git_dir.clone().into_os_string()),
            (
                "GIT_OBJECT_DIRECTORY",
                git_dir.join("objects").into_os_string(),
            ),
            (
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                git_dir.join("objects").into_os_string(),
            ),
        ];
        isolated(test_name!(), &env);
        return;
    }
    assert!(std::env::var_os("GIT_DIR").is_some());
    let fixture = Fixture::standard();
    let repo = Repo::open(&fixture.root()).expect("open");
    assert!(same_path(repo.root(), &fixture.root()));
    assert_eq!(repo.head().expect("head").as_str(), fixture.rev("HEAD"));
    assert!(!repo.is_dirty().expect("clean"));
}

#[test]
fn an_edit_staged_and_matched_by_the_working_tree_is_dirty() {
    isolate!();
    let fixture = Fixture::standard();
    fixture.write("src/app.ts", "staged\n");
    fixture.git(&["add", "src/app.ts"]);
    let repo = Repo::open(&fixture.root()).expect("open");
    assert!(repo.is_dirty().expect("dirty"));
}

#[cfg(target_os = "linux")]
#[test]
fn an_untracked_file_with_a_non_utf8_name_makes_the_worktree_dirty() {
    use std::os::unix::ffi::OsStrExt;
    isolate!();
    let fixture = Fixture::standard();
    let name = std::ffi::OsStr::from_bytes(b"\xff.txt");
    fs::write(fixture.root().join(name), "x\n").expect("write");
    let repo = Repo::open(&fixture.root()).expect("open");
    assert!(repo.is_dirty().expect("dirty"));
}

#[cfg(target_os = "linux")]
#[test]
fn a_worktree_under_a_non_utf8_directory_opens() {
    use std::os::unix::ffi::OsStrExt;
    isolate!();
    let fixture = Fixture::new();
    let root = fixture.root().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    fs::create_dir_all(root.join("sub")).expect("dirs");
    fixture.git_in(&root, &["init", "-q", "-b", "main"]);
    fixture.git_in(&root, &["commit", "-q", "--allow-empty", "-m", "B"]);
    let repo = Repo::open(&root.join("sub")).expect("open");
    assert!(same_path(repo.root(), &root));
    assert_eq!(
        repo.head().expect("head").as_str(),
        fixture.git_in(&root, &["rev-parse", "HEAD"]).trim()
    );
}

#[test]
fn discovery_tells_a_broken_repository_from_one_without_a_worktree() {
    isolate!();
    let fixture = Fixture::standard();
    let bare = fixture.dir.path().join("bare.git");
    fixture.git_in(
        fixture.dir.path(),
        &[
            "init",
            "-q",
            "--bare",
            bare.to_str().expect("utf-8 temp path"),
        ],
    );
    for dir in [bare, fixture.root().join(".git")] {
        let err = Repo::open(&dir).expect_err("no worktree");
        assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
    }

    fixture.git(&["config", "core.bare", "notabool"]);
    let err = Repo::open(&fixture.root()).expect_err("broken config");
    let GitError::Failed { command, stderr } = &err else {
        panic!("expected Failed, got {err:?}");
    };
    assert_eq!(command, "git rev-parse --show-toplevel");
    assert!(stderr.contains("core.bare"), "{stderr}");
}

#[test]
fn a_sha256_repository_resolves_full_object_ids() {
    isolate!();
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main", "--object-format=sha256"]);
    fixture.write("src/app.ts", "B\n");
    fixture.commit("B");
    let b = fixture.rev("HEAD");
    fixture.git(&["checkout", "-q", "-b", "feat"]);
    fixture.write("feat.txt", "F\n");
    fixture.commit("F");
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("main.txt", "M\n");
    fixture.commit("M");
    fixture.git(&["checkout", "-q", "feat"]);

    let repo = Repo::open(&fixture.root()).expect("open");
    let head = repo.head().expect("head");
    let main = repo.resolve_commit("main").expect("main");
    let base = repo.merge_base(&main, &head).expect("merge base");

    assert_eq!(head.as_str().len(), 64);
    assert_eq!(head.as_str(), fixture.rev("HEAD"));
    assert_eq!(main.as_str(), fixture.rev("main"));
    assert_eq!(base.commit.as_str(), b);
    assert_eq!(base.count, 1);
}

#[test]
fn a_branch_with_no_commit_has_an_unborn_head() {
    isolate!();
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    let repo = Repo::open(&fixture.root()).expect("open");
    let err = repo.head().expect_err("unborn");
    assert!(matches!(err, GitError::UnbornHead), "{err:?}");
    assert_eq!(err.to_string(), "HEAD has no commit");
}

#[test]
fn a_revision_that_is_not_a_commit_is_refused() {
    isolate!();
    let fixture = Fixture::standard();
    let repo = Repo::open(&fixture.root()).expect("open");
    let tree = fixture.rev("HEAD^{tree}");

    for rev in ["nope", tree.as_str()] {
        let err = repo.resolve_commit(rev).expect_err("not a commit");
        assert!(
            matches!(&err, GitError::NotACommit { rev: r } if r == rev),
            "{err:?}"
        );
        let message = err.to_string();
        assert!(message.contains(rev), "{message}");
        assert!(message.contains("is not a commit"), "{message}");
    }
}

#[test]
fn a_revision_starting_with_a_dash_is_refused() {
    isolate!();
    let fixture = Fixture::standard();
    let repo = Repo::open(&fixture.root()).expect("open");
    let err = repo.resolve_commit("-main").expect_err("leading dash");
    assert!(
        matches!(&err, GitError::OptionLikeRevision { rev } if rev == "-main"),
        "{err:?}"
    );
    assert!(err.to_string().contains("must not start with -"), "{err}");
}

#[test]
fn a_commit_id_holds_forty_or_sixty_four_lowercase_hex_digits() {
    let sha1 = "a".repeat(40);
    let sha256 = "0123456789abcdef".repeat(4);
    assert_eq!(CommitId::parse(&sha1).expect("sha1").as_str(), sha1);
    assert_eq!(CommitId::parse(&sha256).expect("sha256").as_str(), sha256);
    for bad in [
        "a".repeat(39),
        "a".repeat(41),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(40),
        "g".repeat(40),
        String::new(),
    ] {
        assert!(CommitId::parse(&bad).is_none(), "{bad}");
    }
    assert_eq!(CommitId::parse(&sha1).expect("sha1").to_string(), sha1);
}

#[test]
fn the_core_git_reader_needs_no_parser_or_git_library() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .args([
            "tree",
            "-p",
            "cargo-crap-core",
            "--all-features",
            "--target",
            "all",
            "-e",
            "normal,build,no-proc-macro",
            "--prefix",
            "none",
            "--locked",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|e| panic!("running {cargo} tree: {e}"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).expect("utf-8 tree");

    let banned = ["syn ", "oxc", "git2 ", "libgit2-sys ", "gix", "gitoxide"];
    let hits: Vec<&str> = tree
        .lines()
        .filter(|line| banned.iter().any(|prefix| line.starts_with(prefix)))
        .collect();
    assert!(hits.is_empty(), "the core links {hits:?}");
    assert!(tree.starts_with("cargo-crap-core "), "{tree}");
}

fn five_lines() -> &'static str {
    "a\nb\nc\nd\ne\n"
}

fn changes_fixture() -> (Fixture, Repo, CommitId) {
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    fixture.git(&["config", "core.autocrlf", "false"]);
    fixture.write("src/app.ts", five_lines());
    fixture.write("src/data.ts", "text\n");
    fixture.write(".git/info/exclude", "src/tmp.ts\n");
    fixture.commit("B");
    fixture.git(&["checkout", "-q", "-b", "feat"]);
    let repo = Repo::open(&fixture.root()).expect("open");
    let base = repo.resolve_commit("main").expect("main");
    (fixture, repo, base)
}

fn touching<'a>(
    changes: &'a [Change],
    paths: &[&str],
) -> Vec<&'a Change> {
    let named = |side: &Option<Side>| {
        side.as_ref()
            .is_some_and(|s| paths.iter().any(|p| s.path == Path::new(p)))
    };
    changes
        .iter()
        .filter(|c| named(&c.old) || named(&c.new))
        .collect()
}

fn side<'a>(
    side: Option<&'a Side>,
    path: &str,
) -> &'a Side {
    let side = side.expect("side present");
    assert_eq!(side.path, Path::new(path));
    side
}

fn only<'a>(
    changes: &'a [Change],
    paths: &[&str],
) -> &'a Change {
    let found = touching(changes, paths);
    assert_eq!(found.len(), 1, "{changes:?}");
    found[0]
}

#[test]
fn changed_paths_a_staged_edit_is_modified_with_regular_modes() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/app.ts", "a\nB\nc\nd\ne\n");
    fixture.git(&["add", "src/app.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    assert_eq!(side(change.old.as_ref(), "src/app.ts").mode, Mode::REGULAR);
    assert_eq!(side(change.new.as_ref(), "src/app.ts").mode, Mode::REGULAR);
    assert!(!change.untracked);
    assert_eq!(changes.len(), 1, "{changes:?}");
}

#[test]
fn changed_paths_an_unstaged_edit_is_modified() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/app.ts", "a\nB\nc\nd\ne\n");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    side(change.old.as_ref(), "src/app.ts");
    side(change.new.as_ref(), "src/app.ts");
    assert!(!change.untracked);
}

#[test]
fn changed_paths_a_move_with_one_edited_line_is_renamed() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.git(&["mv", "src/app.ts", "src/main.ts"]);
    fixture.write("src/main.ts", "a\nb\nC\nd\ne\n");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts", "src/main.ts"]);
    assert_eq!(change.status, ChangeStatus::Renamed);
    side(change.old.as_ref(), "src/app.ts");
    side(change.new.as_ref(), "src/main.ts");
}

#[test]
fn changed_paths_git_rm_is_deleted() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.git(&["rm", "-q", "src/app.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::Deleted);
    side(change.old.as_ref(), "src/app.ts");
    assert!(change.new.is_none());
    assert!(!change.untracked);
}

#[test]
fn changed_paths_git_rm_cached_is_an_untracked_modification() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.git(&["rm", "-q", "--cached", "src/app.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    assert_eq!(side(change.old.as_ref(), "src/app.ts").mode, Mode::REGULAR);
    assert_eq!(side(change.new.as_ref(), "src/app.ts").mode, Mode::REGULAR);
    assert!(change.untracked);
    assert_eq!(changes.len(), 1, "{changes:?}");
}

#[test]
fn changed_paths_an_untracked_file_is_added() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/new.ts", "new\n");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/new.ts"]);
    assert_eq!(change.status, ChangeStatus::Added);
    assert!(change.old.is_none());
    let new = side(change.new.as_ref(), "src/new.ts");
    assert_eq!(new.mode, Mode::REGULAR);
    assert!(!new.binary);
    assert!(change.untracked);
}

#[test]
fn changed_paths_an_untracked_ignored_file_is_no_change() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/tmp.ts", "scratch\n");
    let changes = repo.changes(&base).expect("changes");
    assert!(
        touching(&changes, &["src/tmp.ts"]).is_empty(),
        "{changes:?}"
    );
    assert!(changes.is_empty(), "{changes:?}");
}

#[cfg(unix)]
#[test]
fn changed_paths_a_file_replaced_by_a_symbolic_link_is_type_changed() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    let path = fixture.root().join("src/app.ts");
    fs::remove_file(&path).expect("remove");
    std::os::unix::fs::symlink("nowhere", &path).expect("symlink");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::TypeChanged);
    assert_eq!(side(change.old.as_ref(), "src/app.ts").mode, Mode::REGULAR);
    let new = side(change.new.as_ref(), "src/app.ts");
    assert_eq!(new.mode, Mode::SYMLINK);
    assert!(!new.binary);
}

#[test]
fn changed_paths_a_staged_copy_is_added_because_copy_detection_is_off() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.git(&["config", "diff.renames", "copies"]);
    fixture.write("src/copy.ts", five_lines());
    fixture.git(&["add", "src/copy.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/copy.ts"]);
    assert_eq!(change.status, ChangeStatus::Added);
    assert!(change.old.is_none());
    side(change.new.as_ref(), "src/copy.ts");
    assert!(!change.untracked);
    assert_eq!(changes.len(), 1, "{changes:?}");
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[cfg(unix)]
#[test]
fn changed_paths_chmod_x_is_modified_with_an_executable_mode() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    make_executable(&fixture.root().join("src/app.ts"));
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/app.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    assert_eq!(
        side(change.new.as_ref(), "src/app.ts").mode,
        Mode::EXECUTABLE
    );
}

#[cfg(unix)]
#[test]
fn changed_paths_an_untracked_file_takes_its_mode_from_the_file_system() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/run.ts", "run\n");
    make_executable(&fixture.root().join("src/run.ts"));
    fixture.write("src/plain.ts", "plain\n");
    std::os::unix::fs::symlink("plain.ts", fixture.root().join("src/link.ts")).expect("link");
    let changes = repo.changes(&base).expect("changes");
    let mode = |path: &str| side(only(&changes, &[path]).new.as_ref(), path).mode;
    assert_eq!(mode("src/run.ts"), Mode::EXECUTABLE);
    assert_eq!(mode("src/plain.ts"), Mode::REGULAR);
    assert_eq!(mode("src/link.ts"), Mode::SYMLINK);
}

#[test]
fn changed_paths_a_text_file_whose_working_copy_has_a_nul_is_binary_on_the_new_side() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/data.ts", "te\0xt\n");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/data.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    assert!(!side(change.old.as_ref(), "src/data.ts").binary);
    assert!(side(change.new.as_ref(), "src/data.ts").binary);
}

#[test]
fn changed_paths_a_binary_file_whose_working_copy_is_text_is_binary_on_the_old_side() {
    isolate!();
    let (fixture, _, _) = changes_fixture();
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("src/data.ts", "bi\0nary\n");
    fixture.write(".gitattributes", "*.ts diff\n");
    fixture.commit("binary data at B");
    fixture.git(&["checkout", "-q", "feat"]);
    fixture.git(&["reset", "-q", "--hard", "main"]);
    fixture.write("src/data.ts", "text\n");
    let repo = Repo::open(&fixture.root()).expect("open");
    let base = repo.resolve_commit("main").expect("main");
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["src/data.ts"]);
    assert_eq!(change.status, ChangeStatus::Modified);
    assert!(side(change.old.as_ref(), "src/data.ts").binary);
    assert!(!side(change.new.as_ref(), "src/data.ts").binary);
}

#[test]
fn changed_paths_an_untracked_file_with_a_nul_in_its_first_8000_bytes_is_binary() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    let mut early = "x".repeat(7999);
    early.push('\0');
    fixture.write("src/blob.ts", &early);
    let mut late = "x".repeat(8000);
    late.push('\0');
    fixture.write("src/late.ts", &late);
    let changes = repo.changes(&base).expect("changes");
    let blob = only(&changes, &["src/blob.ts"]);
    assert_eq!(blob.status, ChangeStatus::Added);
    assert!(blob.untracked);
    assert!(side(blob.new.as_ref(), "src/blob.ts").binary);
    assert!(!side(only(&changes, &["src/late.ts"]).new.as_ref(), "src/late.ts").binary);
}

#[test]
fn changes_are_sorted_by_path() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    fixture.write("src/zz.ts", "z\n");
    fixture.write("src/aa.ts", "a\n");
    fixture.write("src/data.ts", "edited\n");
    fixture.git(&["rm", "-q", "src/app.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let paths: Vec<&Path> = changes
        .iter()
        .map(|c| {
            c.new
                .as_ref()
                .or(c.old.as_ref())
                .expect("a side")
                .path
                .as_path()
        })
        .collect();
    assert_eq!(
        paths,
        ["src/aa.ts", "src/app.ts", "src/data.ts", "src/zz.ts"].map(Path::new)
    );
}

fn conflicted() -> Fixture {
    let fixture = Fixture::standard();
    fixture.write("src/app.ts", "feat\n");
    fixture.commit("F");
    fixture.git(&["checkout", "-q", "main"]);
    fixture.write("src/app.ts", "main\n");
    fixture.commit("M");
    fixture.git(&["checkout", "-q", "feat"]);
    assert!(
        !fixture.git_ok(&["merge", "-q", "main"]),
        "the merge conflicts"
    );
    fixture
}

#[test]
fn unmerged_entries_conflict_markers_in_the_working_tree() {
    isolate!();
    let fixture = conflicted();
    let repo = Repo::open(&fixture.root()).expect("open");
    assert_eq!(
        repo.unmerged().expect("unmerged"),
        [PathBuf::from("src/app.ts")]
    );
}

#[test]
fn unmerged_entries_an_unstaged_resolution_is_still_unmerged() {
    isolate!();
    let fixture = conflicted();
    fixture.write("src/app.ts", "resolved\n");
    let repo = Repo::open(&fixture.root()).expect("open");
    assert_eq!(
        repo.unmerged().expect("unmerged"),
        [PathBuf::from("src/app.ts")]
    );
}

#[test]
fn unmerged_entries_a_staged_resolution_is_merged() {
    isolate!();
    let fixture = conflicted();
    fixture.write("src/app.ts", "resolved\n");
    fixture.git(&["add", "src/app.ts"]);
    let repo = Repo::open(&fixture.root()).expect("open");
    assert_eq!(repo.unmerged().expect("unmerged"), Vec::<PathBuf>::new());
}

#[test]
fn unmerged_paths_are_distinct_and_sorted() {
    isolate!();
    let fixture = Fixture::standard();
    for (side, text) in [("feat", "f\n"), ("main", "m\n")] {
        if side == "main" {
            fixture.git(&["checkout", "-q", "main"]);
        }
        fixture.write("src/z.ts", text);
        fixture.write("src/a.ts", text);
        fixture.commit(side);
    }
    fixture.git(&["checkout", "-q", "feat"]);
    assert!(
        !fixture.git_ok(&["merge", "-q", "main"]),
        "the merge conflicts"
    );
    let repo = Repo::open(&fixture.root()).expect("open");
    assert_eq!(
        repo.unmerged().expect("unmerged"),
        [PathBuf::from("src/a.ts"), PathBuf::from("src/z.ts")]
    );
}

fn quad(hunks: &[Hunk]) -> Vec<(u32, u32, u32, u32)> {
    hunks
        .iter()
        .map(|h| (h.base_start, h.base_count, h.head_start, h.head_count))
        .collect()
}

#[test]
fn paths_that_contain_pattern_characters_are_read_literally() {
    isolate!();
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    fixture.git(&["config", "core.autocrlf", "false"]);
    fixture.write("src/[x]/page.ts", five_lines());
    fixture.write("src/x/page.ts", five_lines());
    fixture.commit("B");
    fixture.write("src/[x]/page.ts", "a\nB\nc\nd\ne\n");
    fixture.write("src/x/page.ts", "a\nb\nc\nD\ne\n");

    let repo = Repo::open(&fixture.root().join("src")).expect("open");
    let base = repo.head().expect("head");
    let changes = repo.changes(&base).expect("changes");

    let paths: Vec<(ChangeStatus, &Path)> = changes
        .iter()
        .map(|c| (c.status, c.new.as_ref().expect("new side").path.as_path()))
        .collect();
    assert_eq!(
        paths,
        [
            (ChangeStatus::Modified, Path::new("src/[x]/page.ts")),
            (ChangeStatus::Modified, Path::new("src/x/page.ts")),
        ]
    );
    assert_eq!(
        quad(&repo.hunks(&base, &changes[0]).expect("hunks")),
        [(2, 1, 2, 1)]
    );
    assert_eq!(
        quad(&repo.hunks(&base, &changes[1]).expect("hunks")),
        [(4, 1, 4, 1)]
    );
    for path in ["src/[x]/page.ts", "src/x/page.ts"] {
        assert_eq!(
            repo.blob(&base, Path::new(path)).expect("blob"),
            fixture.git_bytes(&["cat-file", "blob", &format!("HEAD:{path}")])
        );
    }
}

fn five() -> (Fixture, Repo, CommitId) {
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    fixture.git(&["config", "core.autocrlf", "false"]);
    fixture.write(".gitattributes", "*.ts -text\n");
    fixture.write("src/five.ts", five_lines());
    fixture.commit("B");
    let repo = Repo::open(&fixture.root()).expect("open");
    let base = repo.head().expect("head");
    (fixture, repo, base)
}

fn five_hunks(edit: impl FnOnce(&Fixture)) -> Vec<(u32, u32, u32, u32)> {
    let (fixture, repo, base) = five();
    edit(&fixture);
    let changes = repo.changes(&base).expect("changes");
    let pair = only(&changes, &["src/five.ts"]);
    quad(&repo.hunks(&base, pair).expect("hunks"))
}

#[test]
fn zero_context_hunks_replacing_line_3() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\nb\nC\nd\ne\n"));
    assert_eq!(hunks, [(3, 1, 3, 1)]);
}

#[test]
fn zero_context_hunks_inserting_two_lines_after_line_2() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\nb\nx\ny\nc\nd\ne\n"));
    assert_eq!(hunks, [(2, 0, 3, 2)]);
}

#[test]
fn zero_context_hunks_deleting_line_2() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\nc\nd\ne\n"));
    assert_eq!(hunks, [(2, 1, 1, 0)]);
}

#[test]
fn zero_context_hunks_deleting_line_1() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "b\nc\nd\ne\n"));
    assert_eq!(hunks, [(1, 1, 0, 0)]);
}

#[test]
fn zero_context_hunks_appending_a_line_after_line_5() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\nb\nc\nd\ne\nf\n"));
    assert_eq!(hunks, [(5, 0, 6, 1)]);
}

#[test]
fn zero_context_hunks_changing_only_the_indentation_of_line_4() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\nb\nc\n  d\ne\n"));
    assert_eq!(hunks, [(4, 1, 4, 1)]);
}

#[test]
fn zero_context_hunks_converting_every_line_ending_to_crlf() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "a\r\nb\r\nc\r\nd\r\ne\r\n"));
    assert_eq!(hunks, [(1, 5, 1, 5)]);
}

#[cfg(unix)]
#[test]
fn zero_context_hunks_a_mode_change_with_no_content_change() {
    isolate!();
    let hunks = five_hunks(|f| {
        make_executable(&f.root().join("src/five.ts"));
    });
    assert_eq!(hunks, []);
}

#[test]
fn zero_context_hunks_git_rm_cached_then_replacing_line_3() {
    isolate!();
    let hunks = five_hunks(|f| {
        f.git(&["rm", "-q", "--cached", "src/five.ts"]);
        f.write("src/five.ts", "a\nb\nC\nd\ne\n");
    });
    assert_eq!(hunks, [(3, 1, 3, 1)]);
}

#[test]
fn zero_context_hunks_of_two_separate_edits_stay_apart() {
    isolate!();
    let hunks = five_hunks(|f| f.write("src/five.ts", "A\nb\nc\nd\nE\n"));
    assert_eq!(hunks, [(1, 1, 1, 1), (5, 1, 5, 1)]);
}

#[test]
fn an_unchanged_reconciled_file_has_no_hunks() {
    isolate!();
    let hunks = five_hunks(|f| {
        f.git(&["rm", "-q", "--cached", "src/five.ts"]);
    });
    assert_eq!(hunks, []);
}

#[test]
fn an_untracked_added_file_is_one_added_hunk() {
    isolate!();
    let (fixture, repo, base) = five();
    fixture.write("src/new.ts", "x\ny\n");
    let changes = repo.changes(&base).expect("changes");
    let added = only(&changes, &["src/new.ts"]);
    assert_eq!(
        quad(&repo.hunks(&base, added).expect("hunks")),
        [(0, 0, 1, 2)]
    );
}

#[test]
fn a_deleted_file_is_one_removed_hunk() {
    isolate!();
    let (fixture, repo, base) = five();
    fixture.git(&["rm", "-q", "src/five.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let deleted = only(&changes, &["src/five.ts"]);
    assert_eq!(
        quad(&repo.hunks(&base, deleted).expect("hunks")),
        [(1, 5, 0, 0)]
    );
}

#[test]
fn hunks_of_a_renamed_file_cover_only_its_edited_lines() {
    isolate!();
    let (fixture, repo, base) = five();
    fixture.git(&["mv", "src/five.ts", "src/six.ts"]);
    fixture.write("src/six.ts", "a\nb\nC\nd\ne\n");

    let changes = repo.changes(&base).expect("changes");
    let renamed = only(&changes, &["src/five.ts", "src/six.ts"]);
    assert_eq!(renamed.status, ChangeStatus::Renamed);

    assert_eq!(
        quad(&repo.hunks(&base, renamed).expect("hunks")),
        [(3, 1, 3, 1)]
    );
    assert_eq!(
        repo.blob(&base, Path::new("src/five.ts")).expect("blob"),
        fixture.git_bytes(&["cat-file", "blob", "HEAD:src/five.ts"])
    );
    assert_eq!(
        repo.blob(&base, Path::new("src/five.ts")).expect("blob"),
        five_lines().as_bytes()
    );
}

#[test]
fn a_blob_missing_at_the_commit_names_the_command() {
    isolate!();
    let (_fixture, repo, base) = five();
    let err = repo
        .blob(&base, Path::new("src/absent.ts"))
        .expect_err("no blob");
    let GitError::Failed { command, .. } = &err else {
        panic!("expected Failed, got {err:?}");
    };
    assert!(command.contains("cat-file blob"), "{command}");
    assert!(command.contains("src/absent.ts"), "{command}");
}

type Reading = (Vec<Change>, Vec<Vec<(u32, u32, u32, u32)>>, Vec<Vec<u8>>);

fn reading(
    configure: impl FnOnce(&Fixture),
    path: &str,
) -> Reading {
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    fixture.git(&["config", "core.autocrlf", "false"]);
    fixture.write(path, "alpha\n    beta\ngamma\n\ndelta\n");
    fixture.write("src/keep.ts", "keep\n");
    fixture.commit("B");
    configure(&fixture);
    fixture.write(path, "alpha\n    beta\n    beta\ngamma\n\n\ndelta\nomega\n");
    let repo = Repo::open(&fixture.root().join("src")).expect("open");
    let base = repo.head().expect("head");
    let changes = repo.changes(&base).expect("changes");
    let hunks = changes
        .iter()
        .map(|c| quad(&repo.hunks(&base, c).expect("hunks")))
        .collect();
    let blobs = changes
        .iter()
        .map(|c| {
            repo.blob(&base, &c.old.as_ref().expect("old").path)
                .expect("blob")
        })
        .collect();
    (changes, hunks, blobs)
}

#[test]
fn the_reader_output_ignores_user_git_configuration() {
    isolate!();
    let path = "src/caf\u{e9} menu/page.ts";
    let configured = reading(
        |f| {
            for (key, value) in [
                ("diff.algorithm", "patience"),
                ("diff.indentHeuristic", "true"),
                ("diff.relative", "true"),
                ("diff.renames", "copies"),
                ("core.quotePath", "true"),
                ("diff.external", "false"),
                ("diff.upper.textconv", "tr a-z A-Z"),
            ] {
                f.git(&["config", key, value]);
            }
            f.write(".git/info/attributes", "*.ts diff=upper\n");
        },
        path,
    );

    let plain = reading(|_| {}, path);

    assert_eq!(configured, plain);
    let (changes, hunks, blobs) = configured;
    assert_eq!(changes.len(), 1, "{changes:?}");
    side(changes[0].old.as_ref(), path);
    side(changes[0].new.as_ref(), path);
    assert_eq!(hunks[0], [(2, 0, 3, 1), (4, 0, 6, 1), (5, 0, 8, 1)]);
    assert_eq!(blobs[0], b"alpha\n    beta\ngamma\n\ndelta\n");
}

#[test]
fn ambient_diff_options_cannot_widen_zero_context_hunks() {
    let widened = [("GIT_DIFF_OPTS", OsString::from("--unified=9"))];
    if std::env::var_os(ISOLATED).is_none() {
        isolated(test_name!(), &widened);
        return;
    }
    assert!(std::env::var_os("GIT_DIFF_OPTS").is_some());
    let (fixture, repo, base) = five();
    fixture.write("src/five.ts", "a\nb\nC\nd\ne\n");
    fixture.write("src/other.ts", "x\n");
    fixture.commit("other");
    fixture.git(&["rm", "-q", "--cached", "src/other.ts"]);
    fixture.write("src/five.ts", "a\nb\nC\nd\ne\n");
    let changes = repo.changes(&base).expect("changes");
    let tracked = only(&changes, &["src/five.ts"]);
    assert_eq!(
        quad(&repo.hunks(&base, tracked).expect("hunks")),
        [(3, 1, 3, 1)]
    );
    let added = only(&changes, &["src/other.ts"]);
    assert!(added.untracked);
    assert_eq!(
        quad(&repo.hunks(&base, added).expect("hunks")),
        [(0, 0, 1, 1)]
    );
}

fn own_temp_files() -> usize {
    let prefix = format!("cargo-crap-git-{}-", std::process::id());
    fs::read_dir(std::env::temp_dir())
        .expect("temp dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .count()
}

#[test]
fn a_reconciled_file_that_vanished_is_a_failed_comparison() {
    isolate!();
    let (fixture, repo, base) = five();
    fixture.git(&["rm", "-q", "--cached", "src/five.ts"]);
    let changes = repo.changes(&base).expect("changes");
    let reconciled = only(&changes, &["src/five.ts"]);
    fs::remove_file(fixture.root().join("src/five.ts")).expect("remove");

    let err = repo.hunks(&base, reconciled).expect_err("vanished");

    let GitError::Failed { command, stderr } = &err else {
        panic!("expected Failed, got {err:?}");
    };
    assert!(command.contains("--no-index"), "{command}");
    assert_ne!(stderr.trim(), "");
    assert_eq!(own_temp_files(), 0);
}

#[cfg(unix)]
#[test]
fn a_reconciled_file_whose_mode_alone_changed_has_no_hunks() {
    isolate!();
    let (fixture, repo, base) = five();
    fixture.git(&["rm", "-q", "--cached", "src/five.ts"]);
    make_executable(&fixture.root().join("src/five.ts"));
    let changes = repo.changes(&base).expect("changes");
    let reconciled = only(&changes, &["src/five.ts"]);
    assert_eq!(quad(&repo.hunks(&base, reconciled).expect("hunks")), []);
    assert_eq!(own_temp_files(), 0);
}

#[test]
fn a_reconciled_root_file_named_dash_is_not_read_as_standard_input() {
    isolate!();
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    fixture.git(&["config", "core.autocrlf", "false"]);
    fixture.write("-", five_lines());
    fixture.commit("B");
    fixture.git(&["rm", "-q", "--cached", "--", "-"]);
    fixture.write("-", "a\nb\nC\nd\ne\n");
    let repo = Repo::open(&fixture.root()).expect("open");
    let base = repo.head().expect("head");
    let changes = repo.changes(&base).expect("changes");
    let dash = only(&changes, &["-"]);
    assert!(dash.untracked);
    assert_eq!(
        quad(&repo.hunks(&base, dash).expect("hunks")),
        [(3, 1, 3, 1)]
    );
}

#[test]
fn a_diff_attribute_does_not_hide_the_hunks_of_two_text_sides() {
    isolate!();
    let (fixture, _, _) = five();
    fixture.write(".gitattributes", "*.ts -diff\n");
    fixture.commit("binary diff attribute");
    let repo = Repo::open(&fixture.root()).expect("open");
    let base = repo.head().expect("head");
    fixture.write("src/five.ts", "a\nb\nc\nD\ne\n");
    let changes = repo.changes(&base).expect("changes");
    let pair = only(&changes, &["src/five.ts"]);
    assert_eq!(
        quad(&repo.hunks(&base, pair).expect("hunks")),
        [(4, 1, 4, 1)]
    );
}

#[test]
fn a_regular_file_is_not_a_worktree() {
    isolate!();
    let fixture = Fixture::standard();
    let err = Repo::open(&fixture.root().join("src/app.ts")).expect_err("a file");
    assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
}

#[test]
fn a_linked_worktree_whose_git_directory_is_gone_is_a_failure() {
    isolate!();
    let fixture = Fixture::standard();
    let linked = fixture.dir.path().join("linked");
    fixture.git(&[
        "worktree",
        "add",
        "-q",
        linked.to_str().expect("utf-8 temp path"),
    ]);
    fs::remove_dir_all(fixture.root().join(".git/worktrees/linked")).expect("remove");
    let err = Repo::open(&linked).expect_err("broken pointer");
    let GitError::Failed { command, stderr } = &err else {
        panic!("expected Failed, got {err:?}");
    };
    assert_eq!(command, "git rev-parse --show-toplevel");
    assert_ne!(stderr.trim(), "");
}

#[test]
fn an_untracked_nested_repository_is_named_without_a_trailing_slash() {
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    let nested = fixture.root().join("vendor");
    fs::create_dir(&nested).expect("nested dir");
    fixture.git_in(&nested, &["init", "-q"]);
    fixture.git_in(&nested, &["commit", "-q", "--allow-empty", "-m", "N"]);
    let changes = repo.changes(&base).expect("changes");
    let change = only(&changes, &["vendor"]);
    assert_eq!(change.status, ChangeStatus::Added);
    let new = side(change.new.as_ref(), "vendor");
    assert_eq!(new.path.as_os_str(), std::ffi::OsStr::new("vendor"));
    assert_eq!(new.mode, Mode::GITLINK);
    assert!(!new.binary);
}

#[cfg(unix)]
#[test]
fn an_untracked_mode_follows_the_owner_execute_bit_as_git_does() {
    use std::os::unix::fs::PermissionsExt;
    isolate!();
    let (fixture, repo, base) = changes_fixture();
    for (name, bits) in [("src/g.ts", 0o654), ("src/u.ts", 0o744)] {
        fixture.write(name, "x\n");
        fs::set_permissions(fixture.root().join(name), fs::Permissions::from_mode(bits))
            .expect("chmod");
    }
    let changes = repo.changes(&base).expect("changes");
    fixture.git(&["add", "src/g.ts", "src/u.ts"]);
    for name in ["src/g.ts", "src/u.ts"] {
        let staged = fixture.git(&["ls-files", "-s", "--", name]);
        let staged =
            u32::from_str_radix(staged.split(' ').next().expect("mode"), 8).expect("octal");
        assert_eq!(
            side(only(&changes, &[name]).new.as_ref(), name).mode,
            Mode(staged),
            "{name}"
        );
    }
}
