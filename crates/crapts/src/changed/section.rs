mod text;

use std::borrow::Cow;
use std::io::Write;
use std::path::Path;

use cargo_crap_core::git::{CommitId, MergeBase};
use cargo_crap_core::report::{Format, Placement, Section, SourceLinks, WriteResult};
use serde::Serialize;
use serde_json::value::RawValue;

use super::verdict::{Change, Counts, Entry, Judged, Verdict};
use crate::Row;
pub use text::{PR_COMMENT_ROWS, utf16_len};
use text::{Text, TextInputs};

const SCHEMA: &str =
    "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/changed-v1.json";

#[derive(Clone, Debug)]
pub struct Comparison {
    /// The `--base` revision as supplied.
    pub rev: String,
    pub commit: CommitId,
    pub merge_base: MergeBase,
    pub head: CommitId,
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SkipReason {
    Binary,
    Excluded,
    OutsidePath,
    Unsupported,
    Submodule,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: SkipReason,
}

/// How a run shows the changed rows. Counts and verdicts never depend on it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Presentation<'a> {
    pub top: Option<usize>,
    pub min: Option<f64>,
    /// The effective `fail-changed`, which makes a GitHub annotation an error.
    pub fail_changed: bool,
    pub summary: bool,
    pub links: Option<&'a SourceLinks>,
    /// Where the invocation sits in the repository, for annotation paths and
    /// source links.
    pub repo_prefix: Option<&'a Path>,
}

#[derive(Debug)]
pub struct ChangedSection {
    json: Box<RawValue>,
    failures: usize,
    text: Text,
}

impl ChangedSection {
    /// # Panics
    /// Never: the object has no map keys that are not strings and no value
    /// whose serializer fails.
    #[must_use]
    pub fn new(
        comparison: &Comparison,
        judged: &Judged<'_>,
        skipped: &[Skipped],
    ) -> Self {
        Self::presented(comparison, judged, skipped, Presentation::default())
    }

    /// # Panics
    /// Never, as for [`ChangedSection::new`].
    #[must_use]
    pub fn presented(
        comparison: &Comparison,
        judged: &Judged<'_>,
        skipped: &[Skipped],
        presentation: Presentation<'_>,
    ) -> Self {
        let mut skipped: Vec<&Skipped> = skipped.iter().collect();
        skipped.sort_by_key(|s| (&s.path, s.reason));
        let shown = shown(&judged.entries, presentation);
        let text = Text::new(&TextInputs {
            shown: &shown,
            counts: judged.counts,
            removed: judged.removed.len(),
            skipped: skipped.len(),
            rev: &comparison.rev,
            merge_base: comparison.merge_base.commit.as_str(),
            fail_changed: presentation.fail_changed,
            summary: presentation.summary,
            links: presentation.links,
            repo_prefix: presentation.repo_prefix,
        });
        let wire = Wire {
            schema: SCHEMA,
            version: env!("CARGO_PKG_VERSION"),
            name_scheme: 1,
            threshold: judged.threshold().get(),
            base: WireBase {
                rev: &comparison.rev,
                commit: comparison.commit.as_str(),
                merge_base: comparison.merge_base.commit.as_str(),
                merge_base_count: comparison.merge_base.count,
                shallow: comparison.merge_base.shallow,
            },
            head: WireHead {
                commit: comparison.head.as_str(),
                dirty: comparison.dirty,
            },
            entries: shown.iter().map(|entry| WireEntry::from(*entry)).collect(),
            removed: judged
                .removed
                .iter()
                .map(|row| WireRemoved::from(*row))
                .collect(),
            skipped,
            counts: judged.counts,
        };
        Self {
            json: serde_json::value::to_raw_value(&wire).expect("the changed object serializes"),
            failures: judged.counts.fail,
            text,
        }
    }

    /// The same section with at most `limit` rows, and never more than
    /// [`PR_COMMENT_ROWS`], in its PR-comment table.
    #[must_use]
    pub fn with_row_limit(
        &self,
        limit: usize,
    ) -> RowLimited<'_> {
        RowLimited {
            section: self,
            limit: limit.min(PR_COMMENT_ROWS),
        }
    }

    /// The rows the text forms show before any row limit.
    #[must_use]
    pub fn shown_rows(&self) -> usize {
        self.text.rows()
    }
}

impl ChangedSection {
    fn write_limited(
        &self,
        format: Format,
        limit: usize,
        out: &mut dyn Write,
    ) -> WriteResult {
        match format {
            Format::Human => self.text.human(out)?,
            Format::Markdown => self.text.markdown(None, out)?,
            Format::PrComment => self.text.markdown(Some(limit), out)?,
            Format::GitHub => self.text.github(out)?,
            Format::Json | Format::Sarif | Format::Shields => {},
        }
        Ok(())
    }
}

impl Section for ChangedSection {
    fn json_key(&self) -> &'static str {
        "changed"
    }

    fn json(&self) -> Box<RawValue> {
        self.json.clone()
    }

    fn write(
        &self,
        format: Format,
        out: &mut dyn Write,
    ) -> WriteResult {
        self.write_limited(format, PR_COMMENT_ROWS, out)
    }

    fn write_pr_comment(
        &self,
        max_rows: usize,
        out: &mut dyn Write,
    ) -> WriteResult {
        self.with_row_limit(max_rows).write(Format::PrComment, out)
    }

    fn placement(&self) -> Placement {
        Placement::Lead
    }

    fn failures(&self) -> Option<usize> {
        Some(self.failures)
    }
}

/// A borrowed [`ChangedSection`] whose PR-comment table holds fewer rows.
#[derive(Clone, Copy, Debug)]
pub struct RowLimited<'s> {
    section: &'s ChangedSection,
    limit: usize,
}

impl Section for RowLimited<'_> {
    fn json_key(&self) -> &'static str {
        "changed"
    }

    fn json(&self) -> Box<RawValue> {
        self.section.json()
    }

    fn write(
        &self,
        format: Format,
        out: &mut dyn Write,
    ) -> WriteResult {
        self.section.write_limited(format, self.limit, out)
    }

    fn write_pr_comment(
        &self,
        max_rows: usize,
        out: &mut dyn Write,
    ) -> WriteResult {
        self.section
            .write_limited(Format::PrComment, self.limit.min(max_rows), out)
    }

    fn placement(&self) -> Placement {
        self.section.placement()
    }

    fn failures(&self) -> Option<usize> {
        self.section.failures()
    }
}

/// `--min` keeps scores at or above it and `--top` the highest scores, with
/// ties broken by file, function and line, as the core selects report rows.
/// The survivors keep the file, line and function order they arrive in.
fn shown<'e, 'r>(
    entries: &'e [Entry<'r>],
    presentation: Presentation,
) -> Vec<&'e Entry<'r>> {
    let mut kept: Vec<usize> = (0..entries.len())
        .filter(|&i| presentation.min.is_none_or(|min| entries[i].crap >= min))
        .collect();
    if let Some(top) = presentation.top {
        kept.sort_by(|&a, &b| {
            entries[b]
                .crap
                .total_cmp(&entries[a].crap)
                .then_with(|| tie_key(&entries[a]).cmp(&tie_key(&entries[b])))
        });
        kept.truncate(top);
        kept.sort_unstable();
    }
    kept.into_iter().map(|i| &entries[i]).collect()
}

fn tie_key<'e>(entry: &'e Entry<'_>) -> (String, &'e str, u32) {
    let head = entry.head;
    (
        head.file.to_string_lossy().replace('\\', "/"),
        &head.name,
        head.span.start.line,
    )
}

fn display(row: &Row) -> Cow<'_, str> {
    row.file.to_string_lossy()
}

#[derive(Serialize)]
struct Wire<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    name_scheme: u8,
    threshold: f64,
    base: WireBase<'a>,
    head: WireHead<'a>,
    entries: Vec<WireEntry<'a>>,
    removed: Vec<WireRemoved<'a>>,
    skipped: Vec<&'a Skipped>,
    counts: Counts,
}

#[derive(Serialize)]
struct WireBase<'a> {
    rev: &'a str,
    commit: &'a str,
    merge_base: &'a str,
    merge_base_count: usize,
    shallow: bool,
}

#[derive(Serialize)]
struct WireHead<'a> {
    commit: &'a str,
    dirty: bool,
}

#[derive(Serialize)]
struct WireEntry<'a> {
    file: Cow<'a, str>,
    function: &'a str,
    line: u32,
    end_line: u32,
    cyclomatic: u32,
    coverage: Option<f64>,
    crap: f64,
    change: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_file: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_cyclomatic: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cyclomatic_delta: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<WireCandidate<'a>>>,
    verdict: &'static str,
    reason: &'a str,
}

impl<'a> From<&'a Entry<'a>> for WireEntry<'a> {
    fn from(entry: &'a Entry<'a>) -> Self {
        let head = entry.head;
        let file = display(head);
        let mut wire = Self {
            file,
            function: &head.name,
            line: head.span.start.line,
            end_line: head.span.end.line,
            cyclomatic: head.cyclomatic,
            coverage: entry.coverage,
            crap: entry.crap,
            change: "new",
            base_file: None,
            base_line: None,
            base_cyclomatic: None,
            cyclomatic_delta: None,
            candidates: None,
            verdict: match entry.judgement.verdict {
                Verdict::Fail => "fail",
                Verdict::Over => "over",
                Verdict::Pass => "pass",
            },
            reason: &entry.judgement.reason,
        };
        match &entry.change {
            Change::New => {},
            Change::Ambiguous(candidates) => {
                wire.change = "ambiguous";
                wire.candidates = Some(
                    candidates
                        .iter()
                        .map(|row| WireCandidate::from(*row))
                        .collect(),
                );
            },
            Change::Modified(base) => {
                wire.change = "modified";
                wire.base_file = Some(display(base)).filter(|path| *path != wire.file);
                wire.base_line = Some(base.span.start.line);
                wire.base_cyclomatic = Some(base.cyclomatic);
                wire.cyclomatic_delta =
                    Some(i64::from(head.cyclomatic) - i64::from(base.cyclomatic));
            },
        }
        wire
    }
}

#[derive(Serialize)]
struct WireCandidate<'a> {
    function: &'a str,
    line: u32,
    cyclomatic: u32,
}

impl<'a> From<&'a Row> for WireCandidate<'a> {
    fn from(row: &'a Row) -> Self {
        Self {
            function: &row.name,
            line: row.span.start.line,
            cyclomatic: row.cyclomatic,
        }
    }
}

#[derive(Serialize)]
struct WireRemoved<'a> {
    file: Cow<'a, str>,
    function: &'a str,
    base_line: u32,
    base_cyclomatic: u32,
}

impl<'a> From<&'a Row> for WireRemoved<'a> {
    fn from(row: &'a Row) -> Self {
        Self {
            file: display(row),
            function: &row.name,
            base_line: row.span.start.line,
            base_cyclomatic: row.cyclomatic,
        }
    }
}
