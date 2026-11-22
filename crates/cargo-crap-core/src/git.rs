//! A mechanical reader of a Git worktree. It runs the `git` executable found
//! on `PATH` and knows nothing about syntax or report rows.
//!
//! Every command runs in the worktree root behind `--literal-pathspecs` and
//! `core.quotePath=false`, so a path such as `src/[slug]/page.ts` names only
//! itself and comes back spelled as committed. Paths inside the worktree
//! must be UTF-8 on every platform. The root itself need not be.

use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

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
    Io {
        path: PathBuf,
        source: io::Error,
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
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
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

impl std::error::Error for GitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> GitError + '_ {
    move |source| GitError::Io {
        path: path.to_path_buf(),
        source,
    }
}

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

    fn bytes(self) -> Result<Vec<u8>, GitError> {
        if self.output.status.success() {
            Ok(self.output.stdout)
        } else {
            Err(self.failed())
        }
    }

    fn stdout(self) -> Result<String, GitError> {
        let command = self.command.clone();
        String::from_utf8(self.bytes()?).map_err(|e| GitError::Unexpected {
            command,
            output: String::from_utf8_lossy(e.as_bytes()).into_owned(),
        })
    }

    fn parsed<T>(
        self,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<T, GitError> {
        let command = self.command.clone();
        parse(&self.stdout()?).map_err(|output| GitError::Unexpected { command, output })
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

// One function with both arms: a separate non-Unix function would be
// mutated and never compiled on the machines that run mutation tests.
#[cfg_attr(unix, expect(clippy::unnecessary_wraps, reason = "fallible off Unix"))]
fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf, Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(std::string::FromUtf8Error::into_bytes)
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

/// Variables that would point Git at another repository, index or object
/// store than the one under `dir`.
const REPOSITORY_OVERRIDES: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
];

fn command(
    program: &str,
    dir: &Path,
) -> Command {
    let mut command = Command::new(program);
    for name in REPOSITORY_OVERRIDES {
        command.env_remove(name);
    }
    // `GIT_DIFF_OPTS=--unified=<n>` would override `-U0`.
    command.env_remove("GIT_DIFF_OPTS");
    command
        .args(["--literal-pathspecs", "-c", "core.quotePath=false"])
        .current_dir(dir);
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
    spawn(command(program, dir), dir, args)
}

fn spawn<I, S>(
    mut git: Command,
    dir: &Path,
    args: I,
) -> Result<Ran, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let command = describe(&args);
    match git.args(&args).output() {
        Ok(output) => Ok(Ran { command, output }),
        Err(e) => Err(spawn_error(&e, dir, command)),
    }
}

fn spawn_error(
    e: &io::Error,
    dir: &Path,
    command: String,
) -> GitError {
    let missing = matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    );
    // A working directory that is missing or a file fails the spawn as if
    // the executable were.
    if missing && !dir.is_dir() {
        GitError::NotAWorktree {
            dir: dir.to_path_buf(),
            stderr: e.to_string(),
        }
    } else if e.kind() == io::ErrorKind::NotFound {
        GitError::GitMissing
    } else {
        GitError::Failed {
            command,
            stderr: e.to_string(),
        }
    }
}

/// Kills and reaps a child whose output was read only in part.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// How many leading bytes of a side are searched for a NUL, as Git does.
const BINARY_PROBE: usize = 8000;

/// A Git file mode as the raw diff prints it, in octal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mode(pub u32);

impl Mode {
    pub const REGULAR: Self = Self(0o100_644);
    pub const EXECUTABLE: Self = Self(0o100_755);
    pub const SYMLINK: Self = Self(0o120_000);
    pub const GITLINK: Self = Self(0o160_000);

    /// A regular file, executable or not: the only kind with text to read.
    #[must_use]
    pub fn is_regular(self) -> bool {
        self == Self::REGULAR || self == Self::EXECUTABLE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeStatus {
    Added,
    Modified,
    Renamed,
    Deleted,
    TypeChanged,
}

/// One side of a change. `binary` is true when the side's first 8000 bytes
/// hold a NUL: the blob at the base for the old side, the working-tree file
/// for the new side, whatever `.gitattributes` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub path: PathBuf,
    pub mode: Mode,
    pub binary: bool,
}

/// A path that differs between a base commit and the working tree.
/// `untracked` marks a new side that is not in the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub status: ChangeStatus,
    pub old: Option<Side>,
    pub new: Option<Side>,
    pub untracked: bool,
}

impl Change {
    fn key(&self) -> &Path {
        let side = self.new.as_ref().or(self.old.as_ref());
        side.map_or(Path::new(""), |s| s.path.as_path())
    }
}

/// A zero-context hunk. A head count of 0 is a deletion point after head
/// line `head_start`, and 0 there is the point before line 1. A base count
/// of 0 removes no base line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hunk {
    pub base_start: u32,
    pub base_count: u32,
    pub head_start: u32,
    pub head_count: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct RawRecord {
    status: u8,
    old_mode: Mode,
    new_mode: Mode,
    paths: Vec<String>,
}

fn parse_mode(octal: &str) -> Option<Mode> {
    let digits = octal.len() == 6 && octal.bytes().all(|b| matches!(b, b'0'..=b'7'));
    digits
        .then(|| u32::from_str_radix(octal, 8).ok())
        .flatten()
        .map(Mode)
}

fn parse_raw(raw: &str) -> Result<Vec<RawRecord>, String> {
    let mut fields = raw.split('\0');
    let mut records = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let record = parse_raw_header(header).ok_or_else(|| header.to_owned())?;
        let wanted = if matches!(record.status, b'R' | b'C') {
            2
        } else {
            1
        };
        let paths: Vec<String> = fields.by_ref().take(wanted).map(str::to_owned).collect();
        if paths.len() != wanted || paths.iter().any(String::is_empty) {
            return Err(header.to_owned());
        }
        records.push(RawRecord { paths, ..record });
    }
    Ok(records)
}

/// `:<old mode> <new mode> <old id> <new id> <status>[score]`.
fn parse_raw_header(header: &str) -> Option<RawRecord> {
    let mut words = header.strip_prefix(':')?.split(' ');
    let old_mode = parse_mode(words.next()?)?;
    let new_mode = parse_mode(words.next()?)?;
    let status = words.nth(2)?.bytes().next()?;
    Some(RawRecord {
        status,
        old_mode,
        new_mode,
        paths: Vec::new(),
    })
}

fn has_nul(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_PROBE).any(|&b| b == 0)
}

fn parse_count(digits: &[u8]) -> Option<u32> {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// `<start>[,<count>]`, with an omitted count read as 1.
fn parse_range(range: &[u8]) -> Option<(u32, u32)> {
    match range.iter().position(|&b| b == b',') {
        Some(comma) => Some((
            parse_count(&range[..comma])?,
            parse_count(&range[comma + 1..])?,
        )),
        None => Some((parse_count(range)?, 1)),
    }
}

/// `@@ -<base range> +<head range> @@[ <heading>]`.
fn parse_hunk_header(line: &[u8]) -> Option<Hunk> {
    let rest = line.strip_prefix(b"@@ -")?;
    let space = rest.iter().position(|&b| b == b' ')?;
    let (base_start, base_count) = parse_range(&rest[..space])?;
    let rest = rest[space..].strip_prefix(b" +")?;
    let space = rest.iter().position(|&b| b == b' ')?;
    let (head_start, head_count) = parse_range(&rest[..space])?;
    let after = rest[space..].strip_prefix(b" @@")?;
    (after.is_empty() || after.starts_with(b" ")).then_some(Hunk {
        base_start,
        base_count,
        head_start,
        head_count,
    })
}

/// Every hunk header of a `-U0` patch. Content lines start with `+`, `-`
/// or a space, so a line that starts with `@@ ` is always a header.
fn parse_hunks(patch: &[u8]) -> Result<Vec<Hunk>, String> {
    patch
        .split(|&b| b == b'\n')
        .filter(|line| line.starts_with(b"@@ "))
        .map(|line| {
            parse_hunk_header(line).ok_or_else(|| String::from_utf8_lossy(line).into_owned())
        })
        .collect()
}

fn parse_unmerged(listing: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = BTreeSet::new();
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        let (_, path) = entry.split_once('\t').ok_or_else(|| entry.to_owned())?;
        paths.insert(PathBuf::from(path));
    }
    Ok(paths.into_iter().collect())
}

/// Git records a regular file as executable by its owner execute bit.
fn is_executable(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o100 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

fn mode_of(meta: &fs::Metadata) -> Mode {
    if meta.file_type().is_symlink() {
        Mode::SYMLINK
    } else if meta.is_dir() {
        Mode::GITLINK
    } else if is_executable(meta) {
        Mode::EXECUTABLE
    } else {
        Mode::REGULAR
    }
}

fn file_prefix(path: &Path) -> io::Result<Vec<u8>> {
    let mut prefix = Vec::new();
    fs::File::open(path)?
        .take(BINARY_PROBE as u64)
        .read_to_end(&mut prefix)?;
    Ok(prefix)
}

/// The base side of a pair whose new side is not in the index, which only
/// `git diff --no-index` can compare. Removed on drop.
#[derive(Debug)]
struct TempFile(PathBuf);

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Names taken before giving up: another process holding this many names
/// with our process id is a stale leftover, not a race.
const TEMP_ATTEMPTS: usize = 100;

impl TempFile {
    fn with(bytes: &[u8]) -> io::Result<Self> {
        Self::in_dir(&std::env::temp_dir(), &NEXT_TEMP, bytes)
    }

    fn in_dir(
        dir: &Path,
        next: &AtomicU64,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // The base blob may be private source in a shared temp directory.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        for _ in 0..TEMP_ATTEMPTS {
            let path = dir.join(temp_name(next.fetch_add(1, Ordering::Relaxed)));
            match options.open(&path) {
                Ok(mut file) => {
                    let temp = Self(path);
                    let wrote = file.write_all(bytes);
                    // Closed before a failed write drops `temp`: Windows
                    // cannot remove a file that is still open.
                    drop(file);
                    wrote?;
                    return Ok(temp);
                },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e),
            }
        }
        Err(io::ErrorKind::AlreadyExists.into())
    }
}

/// `git merge-base --all` lists at least the base `git merge-base` found.
fn best_ancestors(
    command: String,
    all: String,
) -> Result<usize, GitError> {
    match all.lines().count() {
        0 => Err(GitError::Unexpected {
            command,
            output: all,
        }),
        count => Ok(count),
    }
}

fn blob_args(
    commit: &CommitId,
    path: &Path,
) -> [OsString; 3] {
    let mut object = OsString::from(format!("{commit}:"));
    object.push(path);
    [OsString::from("cat-file"), OsString::from("blob"), object]
}

fn temp_name(n: u64) -> String {
    format!("cargo-crap-git-{}-{n}.base", std::process::id())
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Keep diff output independent of user configuration.
const OUTPUT_FLAGS: [&str; 4] = [
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--no-relative",
];

/// `--text` keeps a `binary` or `-diff` attribute from hiding the hunks of
/// a pair whose two sides are text.
const PATCH_FLAGS: [&str; 5] = [
    "--diff-algorithm=myers",
    "--no-indent-heuristic",
    "--text",
    "-U0",
    "--inter-hunk-context=0",
];

const RENAMES: &str = "--find-renames=50%";

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
        // Git exits 128 both outside a repository and on a broken one, so
        // only its message tells them apart: read it untranslated.
        let mut discover = command(program, cwd);
        discover.env("LC_ALL", "C").env_remove("LANGUAGE");
        let ran = spawn(discover, cwd, ["rev-parse", "--show-toplevel"])?;
        if ran.output.status.success() {
            let command = ran.command.clone();
            let mut root = ran.bytes()?;
            if root.last() == Some(&b'\n') {
                root.pop();
            }
            return match path_from_bytes(root) {
                Ok(root) => Ok(Self { root, program }),
                Err(bytes) => Err(GitError::Unexpected {
                    command,
                    output: String::from_utf8_lossy(&bytes).into_owned(),
                }),
            };
        }
        let stderr = ran.stderr();
        // Older Git reports a broken `.git` pointer as "fatal: not a git
        // repository: <path>", which is a failure, not an outside directory.
        let outside = stderr
            .lines()
            .any(|line| line.starts_with("fatal: not a git repository (or any"));
        // A bare repository, or a directory inside `.git`, has a Git
        // directory but no worktree.
        if outside
            || run(program, cwd, ["rev-parse", "--git-dir"])?
                .output
                .status
                .success()
        {
            return Err(GitError::NotAWorktree {
                dir: cwd.to_path_buf(),
                stderr,
            });
        }
        Err(ran.failed())
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
    /// reports with exit status 1.
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
            .bytes()?;
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
        let ran = self.run(["merge-base", "--all", rev.as_str(), head.as_str()])?;
        let command = ran.command.clone();
        Ok(MergeBase {
            commit,
            count: best_ancestors(command, ran.stdout()?)?,
            shallow,
        })
    }

    /// Paths with an index stage above 0. The index is read rather than the
    /// working tree, so a resolution that is written but not staged is still
    /// reported, which a tree-to-working-tree diff cannot detect.
    pub fn unmerged(&self) -> Result<Vec<PathBuf>, GitError> {
        self.run(["ls-files", "--unmerged", "-z"])?
            .parsed(parse_unmerged)
    }

    /// Changes between `base` and the working tree, index edits and
    /// untracked files included, sorted by path. Rename detection is on and
    /// copy detection off.
    pub fn changes(
        &self,
        base: &CommitId,
    ) -> Result<Vec<Change>, GitError> {
        let mut args = vec!["diff"];
        args.extend(OUTPUT_FLAGS);
        args.extend([RENAMES, "--raw", "-z", base.as_str(), "--"]);
        let ran = self.run(args)?;
        let command = ran.command.clone();
        let mut changes = Vec::new();
        for record in ran.parsed(parse_raw)? {
            changes.push(self.change(base, &record, &command)?);
        }
        let mut deleted: HashMap<PathBuf, usize> = changes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.status == ChangeStatus::Deleted)
            .filter_map(|(i, c)| c.old.as_ref().map(|old| (old, i)))
            .filter(|(old, _)| old.mode.is_regular())
            .map(|(old, i)| (old.path.clone(), i))
            .collect();
        let untracked = self
            .run(["ls-files", "--others", "--exclude-standard", "-z"])?
            .stdout()?;
        // An untracked nested repository is listed as `dir/`, and the diff
        // names a tracked one without the slash.
        for path in untracked.split('\0').filter(|p| !p.is_empty()) {
            let path = PathBuf::from(path.trim_end_matches('/'));
            self.add_untracked(&mut changes, &mut deleted, path)?;
        }
        changes.sort_by(|a, b| a.key().cmp(b.key()));
        Ok(changes)
    }

    fn change(
        &self,
        base: &CommitId,
        record: &RawRecord,
        command: &str,
    ) -> Result<Change, GitError> {
        let status = match record.status {
            b'A' => ChangeStatus::Added,
            b'M' => ChangeStatus::Modified,
            b'R' => ChangeStatus::Renamed,
            b'D' => ChangeStatus::Deleted,
            b'T' => ChangeStatus::TypeChanged,
            other => {
                return Err(GitError::Unexpected {
                    command: command.to_owned(),
                    output: format!("{} {}", char::from(other), record.paths.join(" ")),
                });
            },
        };
        let old_path = PathBuf::from(&record.paths[0]);
        let new_path = PathBuf::from(&record.paths[record.paths.len() - 1]);
        let old = match status {
            ChangeStatus::Added => None,
            _ => Some(self.old_side(base, old_path, record.old_mode)?),
        };
        let new = match status {
            ChangeStatus::Deleted => None,
            _ => Some(self.new_side(new_path, record.new_mode)?),
        };
        Ok(Change {
            status,
            old,
            new,
            untracked: false,
        })
    }

    fn old_side(
        &self,
        base: &CommitId,
        path: PathBuf,
        mode: Mode,
    ) -> Result<Side, GitError> {
        let binary = mode.is_regular() && has_nul(&self.blob_prefix(base, &path)?);
        Ok(Side { path, mode, binary })
    }

    fn new_side(
        &self,
        path: PathBuf,
        mode: Mode,
    ) -> Result<Side, GitError> {
        let file = self.root.join(&path);
        let binary = mode.is_regular() && has_nul(&file_prefix(&file).map_err(io_error(&file))?);
        Ok(Side { path, mode, binary })
    }

    /// An untracked regular file where the diff records a regular file as
    /// deleted is that file taken out of the index: one modification.
    fn add_untracked(
        &self,
        changes: &mut Vec<Change>,
        deleted: &mut HashMap<PathBuf, usize>,
        path: PathBuf,
    ) -> Result<(), GitError> {
        let file = self.root.join(&path);
        let meta = fs::symlink_metadata(&file).map_err(io_error(&file))?;
        let new = self.new_side(path, mode_of(&meta))?;
        let reconciled = if new.mode.is_regular() {
            deleted.remove(&new.path)
        } else {
            None
        };
        match reconciled {
            Some(i) => {
                let change = &mut changes[i];
                change.status = ChangeStatus::Modified;
                change.new = Some(new);
                change.untracked = true;
            },
            None => changes.push(Change {
                status: ChangeStatus::Added,
                old: None,
                new: Some(new),
                untracked: true,
            }),
        }
        Ok(())
    }

    /// The zero-context hunks of one change. Both paths go in the pathspec,
    /// since with the new path alone Git reports a renamed file as added. A
    /// new side outside the index is compared with the base blob, or with
    /// nothing when there is no old side, through `git diff --no-index`.
    pub fn hunks(
        &self,
        base: &CommitId,
        change: &Change,
    ) -> Result<Vec<Hunk>, GitError> {
        let mut paths: Vec<&OsStr> = [&change.old, &change.new]
            .into_iter()
            .flatten()
            .map(|side| side.path.as_os_str())
            .collect();
        // With no pathspec the diff would cover the whole repository.
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        if change.untracked {
            return self.untracked_hunks(base, change);
        }
        paths.dedup();
        let mut args: Vec<&OsStr> = vec![OsStr::new("diff")];
        args.extend(OUTPUT_FLAGS.iter().chain(&PATCH_FLAGS).map(OsStr::new));
        args.extend([RENAMES, base.as_str(), "--"].map(OsStr::new));
        args.extend(paths);
        let ran = self.run(args)?;
        let command = ran.command.clone();
        parse_hunks(&ran.bytes()?).map_err(|output| GitError::Unexpected { command, output })
    }

    fn untracked_hunks(
        &self,
        base: &CommitId,
        change: &Change,
    ) -> Result<Vec<Hunk>, GitError> {
        let Some(new) = &change.new else {
            return Ok(Vec::new());
        };
        let before = match &change.old {
            Some(old) => self.blob(base, &old.path)?,
            None => Vec::new(),
        };
        let temp = TempFile::with(&before).map_err(io_error(&std::env::temp_dir()))?;
        // Absolute, so a file named `-` is never read as standard input.
        let after = self.root.join(&new.path);
        let mut args: Vec<&OsStr> = vec![OsStr::new("diff"), OsStr::new("--no-index")];
        args.extend(OUTPUT_FLAGS.iter().chain(&PATCH_FLAGS).map(OsStr::new));
        args.extend([OsStr::new("--"), temp.0.as_os_str(), after.as_os_str()]);
        let ran = self.run(args)?;
        let command = ran.command.clone();
        // `--no-index` exits 1 when the files differ, and also when it cannot
        // read one, which leaves stdout empty and says why on stderr.
        let unreadable = ran.output.stdout.is_empty() && !ran.output.stderr.is_empty();
        match ran.code() {
            Some(0) => Ok(Vec::new()),
            Some(1) if !unreadable => parse_hunks(&ran.output.stdout)
                .map_err(|output| GitError::Unexpected { command, output }),
            _ => Err(ran.failed()),
        }
    }

    /// The bytes Git stores for `path` at `commit`, with no filter or
    /// textconv applied.
    pub fn blob(
        &self,
        commit: &CommitId,
        path: &Path,
    ) -> Result<Vec<u8>, GitError> {
        self.run(blob_args(commit, path))?.bytes()
    }

    /// The first [`BINARY_PROBE`] bytes of a blob, read without holding the
    /// rest in memory.
    fn blob_prefix(
        &self,
        commit: &CommitId,
        path: &Path,
    ) -> Result<Vec<u8>, GitError> {
        let args = blob_args(commit, path);
        let command = describe(&args);
        let mut git = self::command(self.program, &self.root);
        git.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = Reaped(
            git.spawn()
                .map_err(|e| spawn_error(&e, &self.root, command.clone()))?,
        );
        let read_error = |e: io::Error| GitError::Failed {
            command: command.clone(),
            stderr: e.to_string(),
        };
        let mut prefix = Vec::new();
        if let Some(stdout) = child.0.stdout.take() {
            stdout
                .take(BINARY_PROBE as u64)
                .read_to_end(&mut prefix)
                .map_err(read_error)?;
        }
        if !prefix.is_empty() {
            return Ok(prefix);
        }
        let mut stderr = String::new();
        if let Some(mut pipe) = child.0.stderr.take() {
            pipe.read_to_string(&mut stderr).map_err(read_error)?;
        }
        if child.0.wait().map_err(read_error)?.success() {
            Ok(prefix)
        } else {
            Err(GitError::Failed { command, stderr })
        }
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
            assert_ne!(stderr.trim(), "");
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

    #[test]
    fn a_temp_file_skips_names_already_taken_and_is_removed_on_drop() {
        let dir = TempDir::new().expect("temp dir");
        for n in 0..5 {
            fs::write(dir.path().join(temp_name(n)), "taken").expect("taken");
        }
        let next = AtomicU64::new(0);
        let temp = TempFile::in_dir(dir.path(), &next, b"base").expect("temp file");
        assert_eq!(temp.0, dir.path().join(temp_name(5)));
        assert_eq!(fs::read(&temp.0).expect("read"), b"base");
        assert_eq!(
            fs::read(dir.path().join(temp_name(0))).expect("read"),
            b"taken"
        );
        let path = temp.0.clone();
        drop(temp);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_temp_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("temp dir");
        let temp = TempFile::in_dir(dir.path(), &AtomicU64::new(0), b"base").expect("temp file");
        let mode = fs::metadata(&temp.0)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_temp_file_in_a_missing_directory_fails_at_once() {
        let dir = TempDir::new().expect("temp dir");
        let next = AtomicU64::new(0);
        let err = TempFile::in_dir(&dir.path().join("absent"), &next, b"").expect_err("no dir");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(next.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_temp_file_gives_up_when_every_name_is_taken() {
        let dir = TempDir::new().expect("temp dir");
        for n in 0..TEMP_ATTEMPTS as u64 {
            fs::write(dir.path().join(temp_name(n)), "").expect("taken");
        }
        let next = AtomicU64::new(0);
        let err = TempFile::in_dir(dir.path(), &next, b"").expect_err("exhausted");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(next.load(Ordering::Relaxed), TEMP_ATTEMPTS as u64);
        assert!(TempFile::with(b"").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_reaped_child_is_gone_once_dropped() {
        let child = Command::new("sleep").arg("30").spawn().expect("sleep");
        let pid = child.id().to_string();
        drop(Reaped(child));
        let alive = Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .expect("kill -0");
        assert!(!alive.success());
    }

    #[test]
    fn merge_base_all_must_list_a_commit() {
        assert_eq!(
            best_ancestors("git m".to_owned(), "a\nb\n".to_owned()).expect("two"),
            2
        );
        let err = best_ancestors("git m".to_owned(), String::new()).expect_err("none");
        assert!(
            matches!(&err, GitError::Unexpected { command, output } if command == "git m" && output.is_empty()),
            "{err:?}"
        );
    }

    fn committed(files: &[(&str, &[u8])]) -> (TempDir, Repo, CommitId) {
        let dir = TempDir::new().expect("temp dir");
        let empty = dir.path().join("empty.gitconfig");
        fs::write(&empty, "").expect("config");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("repo dir");
        for (path, bytes) in files {
            fs::write(root.join(path), bytes).expect("file");
        }
        for args in [
            &["init", "-q"][..],
            &["add", "-A"],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "B",
            ],
        ] {
            let status = Command::new(GIT)
                .args(args)
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", &empty)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?}");
        }
        let repo = Repo::open(&root).expect("open");
        let head = repo.head().expect("head");
        (dir, repo, head)
    }

    #[test]
    fn a_blob_prefix_holds_at_most_the_probe_length() {
        let big = vec![b'x'; BINARY_PROBE * 3];
        let (_dir, repo, head) = committed(&[("big", &big), ("empty", b""), ("small", b"ab")]);
        assert_eq!(
            repo.blob_prefix(&head, Path::new("big"))
                .expect("big")
                .len(),
            BINARY_PROBE
        );
        assert_eq!(
            repo.blob_prefix(&head, Path::new("small")).expect("small"),
            b"ab"
        );
        assert_eq!(
            repo.blob_prefix(&head, Path::new("empty")).expect("empty"),
            b""
        );
        let err = repo
            .blob_prefix(&head, Path::new("absent"))
            .expect_err("absent");
        let GitError::Failed { command, stderr } = &err else {
            panic!("expected Failed, got {err:?}");
        };
        assert!(command.starts_with("git cat-file blob "), "{command}");
        assert!(command.ends_with(":absent"), "{command}");
        assert_ne!(stderr.trim(), "");
    }

    #[test]
    fn a_change_with_no_side_has_no_hunks() {
        let (_dir, repo, head) = committed(&[("a", b"a\n")]);
        fs::write(repo.root().join("a"), "b\n").expect("edit");
        for untracked in [false, true] {
            let change = Change {
                status: ChangeStatus::Modified,
                old: None,
                new: None,
                untracked,
            };
            assert_eq!(repo.hunks(&head, &change).expect("hunks"), []);
        }
    }

    fn record(
        status: u8,
        old: u32,
        new: u32,
        paths: &[&str],
    ) -> RawRecord {
        RawRecord {
            status,
            old_mode: Mode(old),
            new_mode: Mode(new),
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    #[test]
    fn raw_records_read_their_modes_status_and_paths() {
        let raw = ":100644 100755 aaaa 0000 M\0src/a.ts\0\
                   :100644 100644 aaaa bbbb R080\0old.ts\0new.ts\0\
                   :000000 120000 0000 cccc A\0link\0";
        assert_eq!(
            parse_raw(raw).expect("raw"),
            [
                record(b'M', 0o100_644, 0o100_755, &["src/a.ts"]),
                record(b'R', 0o100_644, 0o100_644, &["old.ts", "new.ts"]),
                record(b'A', 0, 0o120_000, &["link"]),
            ]
        );
        assert_eq!(
            parse_raw(":100644 100644 a b C075\0x\0y\0").expect("copy"),
            [record(b'C', 0o100_644, 0o100_644, &["x", "y"])]
        );
        assert_eq!(parse_raw("").expect("empty"), []);
    }

    #[test]
    fn a_malformed_raw_record_is_refused_with_its_header() {
        for (raw, header) in [
            (":100644 100644 a b R100\0only\0", ":100644 100644 a b R100"),
            (":100644 100644 a b M\0", ":100644 100644 a b M"),
            (":100644 100644 a b M", ":100644 100644 a b M"),
            ("100644 100644 a b M\0x\0", "100644 100644 a b M"),
            (":10064 100644 a b M\0x\0", ":10064 100644 a b M"),
            (":100644 1006440 a b M\0x\0", ":100644 1006440 a b M"),
            (":100648 100644 a b M\0x\0", ":100648 100644 a b M"),
            (":100644 100644 a b\0x\0", ":100644 100644 a b"),
            (":100644 100644 a b \0x\0", ":100644 100644 a b "),
            (":+10644 100644 a b M\0x\0", ":+10644 100644 a b M"),
        ] {
            assert_eq!(parse_raw(raw), Err(header.to_owned()), "{raw:?}");
        }
    }

    #[test]
    fn only_regular_modes_are_regular() {
        assert!(Mode::REGULAR.is_regular());
        assert!(Mode::EXECUTABLE.is_regular());
        assert!(!Mode::SYMLINK.is_regular());
        assert!(!Mode::GITLINK.is_regular());
        assert!(!Mode(0o100_600).is_regular());
    }

    #[test]
    fn a_nul_counts_only_in_the_first_8000_bytes() {
        let mut bytes = vec![b'x'; BINARY_PROBE + 1];
        assert!(!has_nul(&bytes));
        bytes[BINARY_PROBE] = 0;
        assert!(!has_nul(&bytes));
        bytes[BINARY_PROBE - 1] = 0;
        assert!(has_nul(&bytes));
        assert!(has_nul(b"\0"));
        assert!(!has_nul(b""));
    }

    #[test]
    fn a_working_file_is_read_up_to_the_probe_length() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("big");
        fs::write(&path, vec![b'x'; BINARY_PROBE * 2]).expect("write");
        assert_eq!(file_prefix(&path).expect("read").len(), BINARY_PROBE);
        assert!(file_prefix(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn a_directory_has_the_gitlink_mode() {
        let dir = TempDir::new().expect("temp dir");
        let meta = fs::symlink_metadata(dir.path()).expect("metadata");
        assert_eq!(mode_of(&meta), Mode::GITLINK);
    }

    #[test]
    fn hunk_headers_read_omitted_counts_as_one() {
        let hunk = |a, b, c, d| Hunk {
            base_start: a,
            base_count: b,
            head_start: c,
            head_count: d,
        };
        for (line, expected) in [
            (&b"@@ -3 +3 @@ b"[..], hunk(3, 1, 3, 1)),
            (b"@@ -2,0 +3,2 @@", hunk(2, 0, 3, 2)),
            (b"@@ -1,5 +0,0 @@", hunk(1, 5, 0, 0)),
            (b"@@ -12,3 +40 @@ fn x() {", hunk(12, 3, 40, 1)),
        ] {
            assert_eq!(parse_hunk_header(line), Some(expected));
        }
    }

    #[test]
    fn malformed_hunk_headers_are_refused() {
        for line in [
            &b"@@ -3 +3"[..],
            b"@@ -3 +3 @",
            b"@@ -3 +3@@",
            b"@@ -3 3 @@",
            b"@@ 3 +3 @@",
            b"@@ -3, +3 @@",
            b"@@ -,1 +3 @@",
            b"@@ -x +3 @@",
            b"@@ -3 +3,y @@",
            b"@@ -3 +-3 @@",
            b"@@ -3 +99999999999 @@",
            b"@@ - +3 @@",
            b"@@ -3 +3,1,1 @@",
            b"@@ -1 +1 @@garbage",
            b"@@ -1 +1 @@@",
        ] {
            assert_eq!(
                parse_hunk_header(line),
                None,
                "{}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn a_patch_yields_every_header_and_refuses_a_bad_one() {
        let patch = b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-@@ -9 +9 @@\n+a\n@@ -4,0 +5 @@\n+b";
        let hunks = parse_hunks(patch).expect("patch");
        assert_eq!(
            hunks
                .iter()
                .map(|h| (h.base_start, h.base_count, h.head_start, h.head_count))
                .collect::<Vec<_>>(),
            [(1, 1, 1, 1), (4, 0, 5, 1)]
        );
        assert_eq!(parse_hunks(b"@@ -1 +x @@\n"), Err("@@ -1 +x @@".to_owned()));
    }

    #[test]
    fn unmerged_listings_give_distinct_sorted_paths() {
        let listing = "100644 aa 1\tsrc/z.ts\x00100644 bb 2\tsrc/z.ts\x00100644 cc 3\tsrc/a b.ts\0";
        assert_eq!(
            parse_unmerged(listing).expect("listing"),
            [PathBuf::from("src/a b.ts"), PathBuf::from("src/z.ts")]
        );
        assert_eq!(parse_unmerged("").expect("empty"), Vec::<PathBuf>::new());
        assert_eq!(
            parse_unmerged("100644 aa 1 x\0"),
            Err("100644 aa 1 x".to_owned())
        );
    }

    #[test]
    fn a_raw_record_outside_the_five_statuses_is_unexpected() {
        let dir = TempDir::new().expect("temp dir");
        let repo = outside(&dir);
        for status in *b"UCX" {
            let err = repo
                .change(
                    &sha('a'),
                    &record(status, 0o100_644, 0o100_644, &["a", "b"]),
                    "git diff",
                )
                .expect_err("not a change");
            assert!(
                matches!(&err, GitError::Unexpected { command, output } if command == "git diff" && *output == format!("{} a b", char::from(status))),
                "{err:?}"
            );
        }
    }

    #[test]
    fn an_io_error_names_its_path_and_cause() {
        let err = io_error(Path::new("/x/y"))(io::ErrorKind::NotFound.into());
        assert_eq!(
            err.to_string(),
            format!("/x/y: {}", io::Error::from(io::ErrorKind::NotFound))
        );
        assert!(std::error::Error::source(&err).is_some());
        assert!(std::error::Error::source(&GitError::UnbornHead).is_none());
    }

    proptest::proptest! {
        #[test]
        fn a_generated_hunk_header_parses_to_its_four_numbers(
            base_start in proptest::num::u32::ANY,
            base_count in proptest::option::of(proptest::num::u32::ANY),
            head_start in proptest::num::u32::ANY,
            head_count in proptest::option::of(proptest::num::u32::ANY),
            heading in proptest::option::of("[ -~]{0,40}"),
        ) {
            let range = |start: u32, count: Option<u32>| match count {
                Some(count) => format!("{start},{count}"),
                None => start.to_string(),
            };
            let heading = heading.map(|h| format!(" {h}")).unwrap_or_default();
            let line = format!(
                "@@ -{} +{} @@{heading}",
                range(base_start, base_count),
                range(head_start, head_count)
            );
            proptest::prop_assert_eq!(
                parse_hunk_header(line.as_bytes()),
                Some(Hunk {
                    base_start,
                    base_count: base_count.unwrap_or(1),
                    head_start,
                    head_count: head_count.unwrap_or(1),
                })
            );
        }
    }
}
