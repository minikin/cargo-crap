//! A mechanical reader of a Git worktree. It runs the `git` executable found
//! on `PATH` and knows nothing about syntax or report rows.
//!
//! Every command runs in the worktree root behind `--literal-pathspecs` and
//! `core.quotePath=false`, so a path such as `src/[slug]/page.ts` names only
//! itself and comes back spelled as committed. Paths Git prints must be
//! UTF-8, on every platform alike.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GIT: &str = "git";

/// A full commit hash: 40 lowercase hex digits for SHA-1, 64 for SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitId(String);

impl CommitId {
    #[must_use]
    pub fn parse(hex: &str) -> Option<Self> {
        let lower_hex = hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        (matches!(hex.len(), 40 | 64) && lower_hex).then(|| Self(hex.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The merge base Git picks, how many best common ancestors exist (more
/// than one in criss-cross history) and whether the clone is shallow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeBase {
    pub commit: CommitId,
    pub count: usize,
    pub shallow: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GitError {
    GitMissing,
    NotAWorktree {
        dir: PathBuf,
        stderr: String,
    },
    UnbornHead,
    OptionLikeRevision {
        rev: String,
    },
    NotACommit {
        rev: String,
    },
    /// No merge base among the fetched commits of a shallow clone: deeper
    /// history may still hold one.
    ShallowHistory {
        rev: CommitId,
        head: CommitId,
    },
    UnrelatedHistories {
        rev: CommitId,
        head: CommitId,
    },
    Failed {
        command: String,
        stderr: String,
    },
    Unexpected {
        command: String,
        output: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::GitMissing => f.write_str("git executable not found on PATH"),
            Self::NotAWorktree { dir, stderr } => {
                write!(f, "{} is not inside a Git worktree", dir.display())?;
                with_detail(f, stderr)
            },
            Self::UnbornHead => f.write_str("HEAD has no commit"),
            Self::OptionLikeRevision { rev } => write!(f, "revision {rev} must not start with -"),
            Self::NotACommit { rev } => write!(f, "{rev} is not a commit"),
            Self::ShallowHistory { rev, head } => write!(
                f,
                "no merge base of {rev} and {head} in this shallow clone: \
                 check out with fetch-depth: 0 or run git fetch --deepen=<n>"
            ),
            Self::UnrelatedHistories { rev, head } => {
                write!(f, "no merge base of {rev} and {head}: unrelated histories")
            },
            Self::Failed { command, stderr } => {
                write!(f, "{command} failed")?;
                with_detail(f, stderr)
            },
            Self::Unexpected { command, output } => {
                write!(f, "{command} printed unexpected output: {output:?}")
            },
        }
    }
}

fn with_detail(
    f: &mut fmt::Formatter<'_>,
    stderr: &str,
) -> fmt::Result {
    match stderr.trim() {
        "" => Ok(()),
        detail => write!(f, ": {detail}"),
    }
}

impl std::error::Error for GitError {}

struct Ran {
    command: String,
    output: Output,
}

impl Ran {
    fn code(&self) -> Option<i32> {
        self.output.status.code()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    fn failed(self) -> GitError {
        GitError::Failed {
            stderr: self.stderr(),
            command: self.command,
        }
    }

    fn unexpected(
        self,
        output: &str,
    ) -> GitError {
        GitError::Unexpected {
            command: self.command,
            output: output.to_owned(),
        }
    }

    fn stdout(mut self) -> Result<String, GitError> {
        if !self.output.status.success() {
            return Err(self.failed());
        }
        match String::from_utf8(std::mem::take(&mut self.output.stdout)) {
            Ok(text) => Ok(text),
            Err(e) => {
                let lossy = String::from_utf8_lossy(e.as_bytes()).into_owned();
                Err(self.unexpected(&lossy))
            },
        }
    }

    fn line(self) -> Result<String, GitError> {
        let text = self.stdout()?;
        Ok(text.strip_suffix('\n').unwrap_or(&text).to_owned())
    }

    fn commit(self) -> Result<CommitId, GitError> {
        let command = self.command.clone();
        let line = self.line()?;
        CommitId::parse(&line).ok_or(GitError::Unexpected {
            command,
            output: line,
        })
    }
}

fn describe(args: &[OsString]) -> String {
    let mut command = GIT.to_owned();
    for arg in args {
        command.push(' ');
        command.push_str(&arg.to_string_lossy());
    }
    command
}

fn run<I, S>(
    program: &str,
    dir: &Path,
    args: I,
) -> Result<Ran, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let command = describe(&args);
    let output = Command::new(program)
        .args(["--literal-pathspecs", "-c", "core.quotePath=false"])
        .args(&args)
        .current_dir(dir)
        .output();
    match output {
        Ok(output) => Ok(Ran { command, output }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(GitError::GitMissing),
        Err(e) => Err(GitError::Failed {
            command,
            stderr: e.to_string(),
        }),
    }
}

/// An open worktree. Every method runs Git in [`Repo::root`].
#[derive(Clone, Debug)]
pub struct Repo {
    root: PathBuf,
    program: &'static str,
}

impl Repo {
    pub fn open(cwd: &Path) -> Result<Self, GitError> {
        Self::open_with(GIT, cwd)
    }

    fn open_with(
        program: &'static str,
        cwd: &Path,
    ) -> Result<Self, GitError> {
        let not_a_worktree = |stderr: String| GitError::NotAWorktree {
            dir: cwd.to_path_buf(),
            stderr,
        };
        // A missing working directory fails the spawn with NotFound, which
        // would read as a missing git executable.
        if !cwd.is_dir() {
            return Err(not_a_worktree(String::new()));
        }
        let ran = run(program, cwd, ["rev-parse", "--show-toplevel"])?;
        if !ran.output.status.success() {
            return Err(not_a_worktree(ran.stderr()));
        }
        let root = ran.line()?;
        Ok(Self {
            root: PathBuf::from(root),
            program,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn run<I, S>(
        &self,
        args: I,
    ) -> Result<Ran, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run(self.program, &self.root, args)
    }

    /// `None` when `spec` names no commit, which `rev-parse --verify -q`
    /// reports with exit status 1 and nothing else.
    fn verify(
        &self,
        spec: &str,
    ) -> Result<Option<CommitId>, GitError> {
        let ran = self.run(["rev-parse", "--verify", "-q", "--end-of-options", spec])?;
        match ran.code() {
            Some(0) => ran.commit().map(Some),
            Some(1) => Ok(None),
            _ => Err(ran.failed()),
        }
    }

    /// Refuses a revision that starts with `-` before running Git, so it can
    /// never be read as an option.
    pub fn resolve_commit(
        &self,
        rev: &str,
    ) -> Result<CommitId, GitError> {
        if rev.starts_with('-') {
            return Err(GitError::OptionLikeRevision {
                rev: rev.to_owned(),
            });
        }
        self.verify(&format!("{rev}^{{commit}}"))?
            .ok_or_else(|| GitError::NotACommit {
                rev: rev.to_owned(),
            })
    }

    pub fn head(&self) -> Result<CommitId, GitError> {
        self.verify("HEAD^{commit}")?.ok_or(GitError::UnbornHead)
    }

    /// True when the index or the working tree differs from HEAD, or an
    /// untracked file that no ignore rule covers exists.
    pub fn is_dirty(&self) -> Result<bool, GitError> {
        let status = self
            .run(["status", "--porcelain=v1", "-z", "--untracked-files=normal"])?
            .stdout()?;
        Ok(!status.is_empty())
    }

    fn is_shallow(&self) -> Result<bool, GitError> {
        let ran = self.run(["rev-parse", "--is-shallow-repository"])?;
        let command = ran.command.clone();
        match ran.line()?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(GitError::Unexpected {
                command,
                output: other.to_owned(),
            }),
        }
    }

    pub fn merge_base(
        &self,
        rev: &CommitId,
        head: &CommitId,
    ) -> Result<MergeBase, GitError> {
        let shallow = self.is_shallow()?;
        let ran = self.run(["merge-base", rev.as_str(), head.as_str()])?;
        match ran.code() {
            Some(0) => {},
            Some(1) if shallow => {
                return Err(GitError::ShallowHistory {
                    rev: rev.clone(),
                    head: head.clone(),
                });
            },
            Some(1) => {
                return Err(GitError::UnrelatedHistories {
                    rev: rev.clone(),
                    head: head.clone(),
                });
            },
            _ => return Err(ran.failed()),
        }
        let commit = ran.commit()?;
        let all = self
            .run(["merge-base", "--all", rev.as_str(), head.as_str()])?
            .stdout()?;
        Ok(MergeBase {
            commit,
            count: all.lines().count(),
            shallow,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const MISSING: &str = "cargo-crap-no-such-git";

    fn sha(digit: char) -> CommitId {
        CommitId::parse(&digit.to_string().repeat(40)).expect("hex")
    }

    fn outside(dir: &TempDir) -> Repo {
        Repo {
            root: dir.path().to_path_buf(),
            program: GIT,
        }
    }

    #[test]
    fn a_missing_executable_is_git_missing() {
        let dir = TempDir::new().expect("temp dir");
        let err = Repo::open_with(MISSING, dir.path()).expect_err("no git");
        assert!(matches!(err, GitError::GitMissing), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_that_cannot_start_is_a_failed_command() {
        let dir = TempDir::new().expect("temp dir");
        let err = Repo::open_with("/", dir.path()).expect_err("a directory cannot run");
        let GitError::Failed { command, stderr } = &err else {
            panic!("expected Failed, got {err:?}");
        };
        assert_eq!(command, "git rev-parse --show-toplevel");
        assert_ne!(stderr, "");
    }

    #[test]
    fn a_leading_dash_is_refused_before_git_runs() {
        let dir = TempDir::new().expect("temp dir");
        let repo = Repo {
            root: dir.path().to_path_buf(),
            program: MISSING,
        };
        let err = repo.resolve_commit("-x").expect_err("leading dash");
        assert!(
            matches!(&err, GitError::OptionLikeRevision { rev } if rev == "-x"),
            "{err:?}"
        );
        let err = repo.resolve_commit("x-").expect_err("git is missing");
        assert!(matches!(err, GitError::GitMissing), "{err:?}");
    }

    #[test]
    fn a_failing_command_names_itself_and_its_stderr() {
        let dir = TempDir::new().expect("temp dir");
        let repo = outside(&dir);
        for err in [
            repo.is_dirty().expect_err("status outside a repo"),
            repo.head().expect_err("rev-parse outside a repo"),
            repo.resolve_commit("main")
                .expect_err("rev-parse outside a repo"),
            repo.merge_base(&sha('a'), &sha('b'))
                .expect_err("outside a repo"),
        ] {
            let GitError::Failed { command, stderr } = &err else {
                panic!("expected Failed, got {err:?}");
            };
            assert!(command.starts_with("git "), "{command}");
            assert!(stderr.contains("not a git repository"), "{stderr}");
            assert_eq!(
                err.to_string(),
                format!("{command} failed: {}", stderr.trim())
            );
        }
    }

    #[test]
    fn the_described_command_lists_the_arguments_after_git() {
        let args = [OsString::from("merge-base"), OsString::from("--all")];
        assert_eq!(describe(&args), "git merge-base --all");
        assert_eq!(describe(&[]), "git");
    }

    #[test]
    fn every_error_reads_as_a_sentence() {
        let cases = [
            (GitError::GitMissing, "git executable not found on PATH"),
            (
                GitError::NotAWorktree {
                    dir: PathBuf::from("/x"),
                    stderr: " fatal: nope\n".to_owned(),
                },
                "/x is not inside a Git worktree: fatal: nope",
            ),
            (
                GitError::NotAWorktree {
                    dir: PathBuf::from("/x"),
                    stderr: "\n".to_owned(),
                },
                "/x is not inside a Git worktree",
            ),
            (GitError::UnbornHead, "HEAD has no commit"),
            (
                GitError::OptionLikeRevision {
                    rev: "-m".to_owned(),
                },
                "revision -m must not start with -",
            ),
            (
                GitError::NotACommit {
                    rev: "nope".to_owned(),
                },
                "nope is not a commit",
            ),
            (
                GitError::ShallowHistory {
                    rev: sha('a'),
                    head: sha('b'),
                },
                &format!(
                    "no merge base of {} and {} in this shallow clone: check out with \
                     fetch-depth: 0 or run git fetch --deepen=<n>",
                    sha('a'),
                    sha('b')
                ),
            ),
            (
                GitError::UnrelatedHistories {
                    rev: sha('a'),
                    head: sha('b'),
                },
                &format!(
                    "no merge base of {} and {}: unrelated histories",
                    sha('a'),
                    sha('b')
                ),
            ),
            (
                GitError::Failed {
                    command: "git x".to_owned(),
                    stderr: String::new(),
                },
                "git x failed",
            ),
            (
                GitError::Unexpected {
                    command: "git x".to_owned(),
                    output: "odd".to_owned(),
                },
                "git x printed unexpected output: \"odd\"",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    fn ran(
        code: i32,
        stdout: &[u8],
    ) -> Ran {
        let status = Command::new(GIT)
            .args(["-c", &format!("alias.exit=!exit {code}"), "exit"])
            .output()
            .expect("run git")
            .status;
        Ran {
            command: "git probe".to_owned(),
            output: Output {
                status,
                stdout: stdout.to_vec(),
                stderr: b"why\n".to_vec(),
            },
        }
    }

    #[test]
    fn output_that_is_not_a_commit_or_utf8_is_unexpected() {
        let err = ran(0, b"HEAD\n").commit().expect_err("not a hash");
        assert!(
            matches!(&err, GitError::Unexpected { command, output } if command == "git probe" && output == "HEAD"),
            "{err:?}"
        );
        let err = ran(0, b"\xff\n").stdout().expect_err("not utf-8");
        assert!(
            matches!(&err, GitError::Unexpected { output, .. } if output == "\u{fffd}\n"),
            "{err:?}"
        );
    }

    #[test]
    fn a_line_drops_one_final_newline() {
        assert_eq!(ran(0, b"a b\n").line().expect("line"), "a b");
        assert_eq!(ran(0, b"a\n\n").line().expect("line"), "a\n");
        assert_eq!(ran(0, b"a").line().expect("line"), "a");
    }

    #[test]
    fn a_nonzero_exit_fails_with_its_stderr() {
        let err = ran(3, b"").stdout().expect_err("exit 3");
        assert!(
            matches!(&err, GitError::Failed { command, stderr } if command == "git probe" && stderr == "why\n"),
            "{err:?}"
        );
        assert_eq!(ran(3, b"").code(), Some(3));
    }
}
