use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use cargo_crap_core::git::{Change, CommitId, GitError, Hunk, Mode, Repo, Side};

use super::section::{Comparison, SkipReason, Skipped};
use crate::discover::{DiscoveryScope, Placement};
use crate::{FileAnalysis, InputError, Side as Version, analyze};

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
    pub(crate) heads: HashMap<PathBuf, FileAnalysis>,
    /// Changed files whose head side was skipped, a binary one among them:
    /// no pass may decode them.
    pub(crate) skipped_heads: BTreeSet<PathBuf>,
}

#[derive(Debug)]
struct Eligible {
    path: PathBuf,
    display: PathBuf,
}

#[derive(Debug)]
struct Planned {
    base: Option<Eligible>,
    head: Option<Eligible>,
    /// Hunks are read only for a pair of eligible sides of one change.
    change: Option<Change>,
}

impl Planned {
    fn key(&self) -> &Path {
        let side = self.head.as_ref().or(self.base.as_ref());
        side.map_or(Path::new(""), |side| side.display.as_path())
    }
}

/// The `--base` comparison up to strict analysis: the repository state, the
/// changed paths placed against the scored file set, their hunks and both
/// sides' rows.
///
/// # Errors
/// A Git failure, an unresolved merge, an unreadable side or a side that
/// does not parse cleanly.
pub(crate) fn select(
    scope: &DiscoveryScope,
    cwd: &Path,
    rev: &str,
) -> Result<Selection, InputError> {
    let repo = Repo::open(cwd).map_err(|e| base_error(&e))?;
    let head = repo.head().map_err(|e| base_error(&e))?;
    let commit = repo.resolve_commit(rev).map_err(|e| base_error(&e))?;
    let merge_base = repo
        .merge_base(&commit, &head)
        .map_err(|e| base_error(&e))?;
    let unmerged = repo.unmerged().map_err(|e| base_error(&e))?;
    if !unmerged.is_empty() {
        return Err(unresolved(scope, repo.root(), &unmerged));
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
            repo.root(),
            change,
            &mut Outcome {
                planned: &mut planned,
                skipped: &mut skipped,
                skipped_heads: &mut skipped_heads,
            },
        )?;
    }
    planned.sort_by(|a, b| a.key().cmp(b.key()));
    let read = read_sides(&repo, &merge_base.commit, planned)?;
    let (files, heads) = analyze_sides(read)?;
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

fn plan(
    scope: &DiscoveryScope,
    root: &Path,
    change: Change,
    outcome: &mut Outcome<'_>,
) -> Result<(), InputError> {
    let base = match &change.old {
        Some(side) => eligible(scope, root, side, false, outcome)?,
        None => None,
    };
    let head = match &change.new {
        Some(side) => eligible(scope, root, side, true, outcome)?,
        None => None,
    };
    match (base, head) {
        (Some(base), Some(head)) => {
            outcome.planned.push(Planned {
                base: Some(base),
                head: Some(head),
                change: Some(change),
            });
        },
        (base, head) => {
            for (base, head) in [(base, None), (None, head)] {
                if base.is_some() || head.is_some() {
                    outcome.planned.push(Planned {
                        base,
                        head,
                        change: None,
                    });
                }
            }
        },
    }
    Ok(())
}

fn eligible(
    scope: &DiscoveryScope,
    root: &Path,
    side: &Side,
    is_head: bool,
    outcome: &mut Outcome<'_>,
) -> Result<Option<Eligible>, InputError> {
    let absolute = root.join(&side.path);
    let display = scope.display(&absolute);
    let reason = match side.mode {
        mode if mode.is_regular() => {
            let placement = if is_head {
                scope.place_present(&absolute)
            } else {
                scope.place_historical(&absolute)?
            };
            placement_reason(placement, side.binary)
        },
        Mode::SYMLINK => Some(SkipReason::Symlink),
        Mode::GITLINK => Some(SkipReason::Submodule),
        _ => Some(SkipReason::Unsupported),
    };
    let Some(reason) = reason else {
        return Ok(Some(Eligible {
            path: side.path.clone(),
            display,
        }));
    };
    if is_head {
        outcome.skipped_heads.insert(display.clone());
    }
    outcome
        .skipped
        .insert((display.to_string_lossy().into_owned(), reason));
    Ok(None)
}

fn placement_reason(
    placement: Placement,
    binary: bool,
) -> Option<SkipReason> {
    match placement {
        Placement::Unsupported => Some(SkipReason::Unsupported),
        Placement::OutsidePath => Some(SkipReason::OutsidePath),
        Placement::Excluded => Some(SkipReason::Excluded),
        Placement::Scored if binary => Some(SkipReason::Binary),
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
            let head = planned
                .head
                .map(|side| {
                    let bytes = std::fs::read(repo.root().join(&side.path))
                        .map_err(|e| InputError::at(&side.display, e))?;
                    Ok::<_, InputError>(ReadSide {
                        display: side.display,
                        bytes,
                    })
                })
                .transpose()?;
            Ok(Read { base, head, edits })
        })
        .collect()
}

fn analyze_sides(
    read: Vec<Read>
) -> Result<(Vec<SelectedFile>, HashMap<PathBuf, FileAnalysis>), InputError> {
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
                let analysis = analyze(
                    &side.display,
                    &side.bytes,
                    crate::Mode::Strict(Version::Head),
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
