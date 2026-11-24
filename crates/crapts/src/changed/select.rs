use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use cargo_crap_core::git::{Change, CommitId, GitError, Hunk, Mode, Repo, Side};

use super::section::{Comparison, SkipReason, Skipped};
use crate::analyze::{Capture, HeadAnalysis};
use crate::discover::{DiscoveryScope, Placement};
use crate::{FileAnalysis, InputError, Side as Version, analyze};

type Heads = HashMap<PathBuf, HeadAnalysis>;

#[derive(Debug)]
pub(crate) enum FileEdits {
    Added,
    Hunks(Vec<Hunk>),
}

#[derive(Debug)]
pub(crate) struct SelectedFile {
    pub(crate) base: Option<FileAnalysis>,
    /// The display path of the head side, whose strict analysis is in
    /// [`Selection::heads`].
    pub(crate) head: Option<PathBuf>,
    pub(crate) edits: FileEdits,
}

#[derive(Debug)]
pub(crate) struct Selection {
    pub(crate) comparison: Comparison,
    pub(crate) skipped: Vec<Skipped>,
    /// In display-path order.
    pub(crate) files: Vec<SelectedFile>,
    pub(crate) heads: Heads,
    /// Changed files whose head side was skipped, a binary one among them:
    /// no pass may decode them.
    pub(crate) skipped_heads: BTreeSet<PathBuf>,
}

#[derive(Debug)]
struct BaseSide {
    path: PathBuf,
    display: PathBuf,
}

/// The bytes read once for both the binary test and strict analysis.
#[derive(Debug)]
struct HeadSide {
    display: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Planned {
    base: Option<BaseSide>,
    head: Option<HeadSide>,
    change: Option<Change>,
}

impl Planned {
    fn key(&self) -> &Path {
        match (&self.head, &self.base) {
            (Some(head), _) => &head.display,
            (None, Some(base)) => &base.display,
            (None, None) => Path::new(""),
        }
    }
}

/// # Errors
/// A Git failure, an unresolved merge, an unreadable side or a side that
/// does not parse cleanly.
pub(crate) fn select(
    scope: &DiscoveryScope,
    cwd: &Path,
    rev: &str,
    capture: Capture,
) -> Result<Selection, InputError> {
    let repo = Repo::open(cwd).map_err(|e| base_error(&e))?;
    // Discovery holds canonical paths, which on Windows carry a verbatim
    // prefix that Git's own spelling of the root lacks.
    let root = repo
        .root()
        .canonicalize()
        .map_err(|e| InputError::at(repo.root(), e))?;
    let head = repo.head().map_err(|e| base_error(&e))?;
    let commit = repo.resolve_commit(rev).map_err(|e| base_error(&e))?;
    let merge_base = repo
        .merge_base(&commit, &head)
        .map_err(|e| base_error(&e))?;
    let unmerged = repo.unmerged().map_err(|e| base_error(&e))?;
    if !unmerged.is_empty() {
        return Err(unresolved(scope, &root, &unmerged));
    }
    let dirty = repo.is_dirty().map_err(|e| base_error(&e))?;
    let changes = repo
        .changes(&merge_base.commit)
        .map_err(|e| base_error(&e))?;
    let mut skipped = BTreeSet::new();
    let mut skipped_heads = BTreeSet::new();
    let mut planned = Vec::new();
    for change in changes {
        plan(
            scope,
            &root,
            change,
            &mut Outcome {
                planned: &mut planned,
                skipped: &mut skipped,
                skipped_heads: &mut skipped_heads,
            },
        )?;
    }
    planned.sort_by_cached_key(|planned| {
        planned
            .key()
            .to_string_lossy()
            .encode_utf16()
            .collect::<Vec<_>>()
    });
    let read = read_sides(&repo, &merge_base.commit, planned)?;
    let (files, heads) = analyze_sides(read, capture)?;
    Ok(Selection {
        comparison: Comparison {
            rev: rev.to_owned(),
            commit,
            merge_base,
            head,
            dirty,
        },
        skipped: skipped
            .into_iter()
            .map(|(path, reason)| Skipped { path, reason })
            .collect(),
        files,
        heads,
        skipped_heads,
    })
}

fn base_error(error: &GitError) -> InputError {
    match error {
        GitError::NotAWorktree { .. } => {
            InputError::message(format!("--base needs a Git worktree: {error}"))
        },
        _ => InputError::message(format!("--base: {error}")),
    }
}

fn unresolved(
    scope: &DiscoveryScope,
    root: &Path,
    paths: &[PathBuf],
) -> InputError {
    let names: Vec<String> = paths
        .iter()
        .map(|path| scope.display(&root.join(path)).display().to_string())
        .collect();
    InputError::message(format!(
        "--base: unresolved merge in {}: resolve and stage the conflicts first",
        names.join(", ")
    ))
}

struct Outcome<'a> {
    planned: &'a mut Vec<Planned>,
    skipped: &'a mut BTreeSet<(String, SkipReason)>,
    skipped_heads: &'a mut BTreeSet<PathBuf>,
}

impl Outcome<'_> {
    fn skip(
        &mut self,
        display: PathBuf,
        reason: SkipReason,
        is_head: bool,
    ) {
        self.skipped
            .insert((display.to_string_lossy().into_owned(), reason));
        if is_head {
            self.skipped_heads.insert(display);
        }
    }
}

fn plan(
    scope: &DiscoveryScope,
    root: &Path,
    change: Change,
    outcome: &mut Outcome<'_>,
) -> Result<(), InputError> {
    let base = change
        .old
        .as_ref()
        .and_then(|side| base_side(scope, root, side, outcome));
    let head = match &change.new {
        Some(side) => head_side(scope, root, side, outcome)?,
        None => None,
    };
    match (base, head) {
        (None, None) => {},
        (Some(base), Some(head)) => outcome.planned.push(Planned {
            base: Some(base),
            head: Some(head),
            change: Some(change),
        }),
        (base, head) => outcome.planned.push(Planned {
            base,
            head,
            change: None,
        }),
    }
    Ok(())
}

fn base_side(
    scope: &DiscoveryScope,
    root: &Path,
    side: &Side,
    outcome: &mut Outcome<'_>,
) -> Option<BaseSide> {
    let absolute = root.join(&side.path);
    let display = scope.display(&absolute);
    let placement = scope.place_historical(&absolute);
    if let Some(reason) = skip_reason(side, placement) {
        outcome.skip(display, reason, false);
        return None;
    }
    Some(BaseSide {
        path: side.path.clone(),
        display,
    })
}

fn head_side(
    scope: &DiscoveryScope,
    root: &Path,
    side: &Side,
    outcome: &mut Outcome<'_>,
) -> Result<Option<HeadSide>, InputError> {
    let absolute = root.join(&side.path);
    let display = scope.display(&absolute);
    let placement = scope.place_present(&absolute);
    if let Some(reason) = skip_reason(side, placement) {
        outcome.skip(display, reason, true);
        return Ok(None);
    }
    let bytes = std::fs::read(&absolute).map_err(|e| InputError::at(&display, e))?;
    // The file may have changed since the reader looked at it.
    if bytes.iter().take(8000).any(|&byte| byte == 0) {
        outcome.skip(display, SkipReason::Binary, true);
        return Ok(None);
    }
    Ok(Some(HeadSide { display, bytes }))
}

fn skip_reason(
    side: &Side,
    placement: Placement,
) -> Option<SkipReason> {
    match side.mode {
        mode if mode.is_regular() => {},
        Mode::SYMLINK => return Some(SkipReason::Symlink),
        Mode::GITLINK => return Some(SkipReason::Submodule),
        _ => return Some(SkipReason::Unsupported),
    }
    placement_reason(placement).or_else(|| side.binary.then_some(SkipReason::Binary))
}

fn placement_reason(placement: Placement) -> Option<SkipReason> {
    match placement {
        Placement::Unsupported => Some(SkipReason::Unsupported),
        Placement::OutsidePath => Some(SkipReason::OutsidePath),
        Placement::Excluded => Some(SkipReason::Excluded),
        Placement::Scored => None,
    }
}

struct ReadSide {
    display: PathBuf,
    bytes: Vec<u8>,
}

struct Read {
    base: Option<ReadSide>,
    head: Option<ReadSide>,
    edits: FileEdits,
}

fn read_sides(
    repo: &Repo,
    merge_base: &CommitId,
    planned: Vec<Planned>,
) -> Result<Vec<Read>, InputError> {
    planned
        .into_iter()
        .map(|planned| {
            let edits = match &planned.change {
                Some(change) => {
                    FileEdits::Hunks(repo.hunks(merge_base, change).map_err(|e| base_error(&e))?)
                },
                None => FileEdits::Added,
            };
            let base = planned
                .base
                .map(|side| {
                    let bytes = repo
                        .blob(merge_base, &side.path)
                        .map_err(|e| base_error(&e))?;
                    Ok::<_, InputError>(ReadSide {
                        display: side.display,
                        bytes,
                    })
                })
                .transpose()?;
            let head = planned.head.map(|side| ReadSide {
                display: side.display,
                bytes: side.bytes,
            });
            Ok(Read { base, head, edits })
        })
        .collect()
}

fn analyze_sides(
    read: Vec<Read>,
    capture: Capture,
) -> Result<(Vec<SelectedFile>, Heads), InputError> {
    let mut files = Vec::with_capacity(read.len());
    let mut heads = HashMap::new();
    for read in read {
        let base = read
            .base
            .map(|side| {
                analyze(
                    &side.display,
                    &side.bytes,
                    crate::Mode::Strict(Version::Base),
                )
            })
            .transpose()?;
        let head = match read.head {
            Some(side) => {
                let analysis = HeadAnalysis::read(
                    &side.display,
                    side.bytes,
                    crate::Mode::Strict(Version::Head),
                    capture,
                )?;
                heads.insert(side.display.clone(), analysis);
                Some(side.display)
            },
            None => None,
        };
        files.push(SelectedFile {
            base,
            head,
            edits: read.edits,
        });
    }
    Ok((files, heads))
}
