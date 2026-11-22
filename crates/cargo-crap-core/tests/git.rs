//! The core Git reader against temporary repositories.

use cargo_crap_core::git::{CommitId, GitError, Repo};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

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
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", &self.empty_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap_or_else(|e| panic!("running git {args:?}: {e}"));
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

    /// `B` on main with `src/app.ts`, and `feat` checked out at `B`.
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
    // Given the repository with one more commit T on main after B and an
    // uncommitted edit on feat
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

    // When the reader opens the worktree from src/ and resolves "main"
    let repo = Repo::open(&fixture.root().join("src")).expect("open");
    let head = repo.head().expect("head");
    let main = repo.resolve_commit("main").expect("resolve main");

    // Then the root is the repository root, the revision commit equals
    // git rev-parse main
    assert!(same_path(repo.root(), &fixture.root()));
    assert!(repo.root().is_absolute());
    assert_eq!(main.as_str(), fixture.rev("main"));

    // And the merge base is B with count 1 and shallow false
    let base = repo.merge_base(&main, &head).expect("merge base");
    assert_eq!(base.commit.as_str(), b);
    assert_eq!(base.count, 1);
    assert!(!base.shallow);

    // And the head commit equals git rev-parse HEAD and dirty is true
    assert_eq!(head.as_str(), fixture.rev("HEAD"));
    assert!(repo.is_dirty().expect("dirty"));

    // And dirty is false once the edit is reverted and no untracked
    // unignored file exists
    fixture.git(&["checkout", "--", "src/app.ts"]);
    fixture.write(".git/info/exclude", "ignored.txt\n");
    fixture.write("ignored.txt", "not a change\n");
    assert!(!repo.is_dirty().expect("clean"));
    fixture.write("untracked.txt", "a change\n");
    assert!(repo.is_dirty().expect("untracked is dirty"));
}

#[test]
fn criss_cross_history_uses_gits_merge_base_and_records_the_count() {
    // Given branches main and feat that merged each other after diverging
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

    // When the reader computes the merge base of main and HEAD
    let repo = Repo::open(&fixture.root()).expect("open");
    let main = repo.resolve_commit("main").expect("main");
    let head = repo.head().expect("head");
    let base = repo.merge_base(&main, &head).expect("merge base");

    // Then the commit equals the output of git merge-base main HEAD and
    // the count is 2
    assert_eq!(
        base.commit.as_str(),
        fixture.git(&["merge-base", "main", "HEAD"]).trim()
    );
    assert_eq!(base.count, 2);
    assert!(!base.shallow);
}

#[test]
fn a_merge_base_found_in_a_shallow_clone_is_used_and_marked_shallow() {
    // Given a clone made with --depth 3 --no-single-branch in which main
    // and feat share fetched ancestor B
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

    // When the reader computes the merge base of origin/main and HEAD
    let repo = Repo::open(&clone).expect("open clone");
    let main = repo.resolve_commit("origin/main").expect("origin/main");
    let head = repo.head().expect("head");
    let base = repo.merge_base(&main, &head).expect("merge base");

    // Then the commit is B and shallow is true
    assert_eq!(base.commit.as_str(), b);
    assert_eq!(base.count, 1);
    assert!(base.shallow);
}

#[test]
fn a_shallow_clone_without_a_shared_fetched_commit_is_a_shallow_history_error() {
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
    let fixture = Fixture::new();
    let err = Repo::open(&fixture.root().join("absent")).expect_err("missing dir");
    assert!(matches!(err, GitError::NotAWorktree { .. }), "{err:?}");
}

#[test]
fn a_branch_with_no_commit_has_an_unborn_head() {
    let fixture = Fixture::new();
    fixture.git(&["init", "-q", "-b", "main"]);
    let repo = Repo::open(&fixture.root()).expect("open");
    let err = repo.head().expect_err("unborn");
    assert!(matches!(err, GitError::UnbornHead), "{err:?}");
    assert_eq!(err.to_string(), "HEAD has no commit");
}

#[test]
fn a_revision_that_is_not_a_commit_is_refused() {
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
    // Given the workspace manifests
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    // When cargo tree runs
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

    // Then the tree contains no syn, no oxc package and no Git library crate
    let banned = ["syn ", "oxc", "git2 ", "libgit2-sys ", "gix", "gitoxide"];
    let hits: Vec<&str> = tree
        .lines()
        .filter(|line| banned.iter().any(|prefix| line.starts_with(prefix)))
        .collect();
    assert!(hits.is_empty(), "the core links {hits:?}");
    assert!(tree.starts_with("cargo-crap-core "), "{tree}");
}
