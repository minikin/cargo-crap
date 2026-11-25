//! `cargo crap` — CLI entrypoint.
//!
//! Invoked two ways:
//! - Directly as `cargo-crap [args...]`.
//! - As a cargo subcommand `cargo crap [args...]`, in which case cargo
//!   invokes us as `cargo-crap crap [args...]`. We strip that leading
//!   `crap` argument below.

use anyhow::{Context, Result, bail};
use cargo_crap::{
    cache::{
        complexity::{FileRecord, Session},
        target::target_dir,
    },
    complexity,
    coverage::{self, FileCoverage},
    delta::{Baseline, load_baseline_with_weight},
    duplicates,
    duplicates::compare::DuplicatePair,
    duplicates::triage::provider::{Provider, default_provider},
    duplicates::triage::verdict::Assessment,
    filter::AllowSet,
    merge::{MissingCoveragePolicy, ScopeDiagnostics, SortOrder, merge},
    report::{Format, SourceLinks, TriagedBy, output_width, set_color_enabled},
    run::{self, BaselineFilter, PreparedBaseline, RenderExtras, Scope},
    score::DEFAULT_THRESHOLD,
};
use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "cargo-crap",
    about = "Compute the CRAP (Change Risk Anti-Patterns) metric for Rust projects.",
    long_about = None,
    version
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the bools come from clap-derived `--flag` switches (--workspace, --summary, --fail-above, --fail-regression, --no-default-excludes, --show-unchanged, --all-rows); not a struct-design smell"
)]
struct Cli {
    /// Path to an LCOV coverage file (e.g. from `cargo llvm-cov --lcov --output-path lcov.info`).
    ///
    /// If omitted, every function is scored as if it had 0% coverage. That is
    /// useful for a first look at complexity distribution but is not a real
    /// CRAP run.
    #[arg(long, value_name = "FILE")]
    lcov: Option<PathBuf>,

    /// Root directory to analyze. Defaults to the current directory.
    #[arg(long, value_name = "DIR", default_value = ".")]
    path: PathBuf,

    /// Analyze all workspace members discovered via `cargo metadata`.
    ///
    /// When set, `--path` is ignored and every member crate's root is walked.
    /// Useful for monorepos: one command, one LCOV file, full workspace picture.
    #[arg(long)]
    workspace: bool,

    /// Analyze only the named workspace member(s), discovered via
    /// `cargo metadata`. Repeatable, cargo-style: `-p core -p api`.
    ///
    /// One invocation, one LCOV parse, one report, one gate decision over
    /// exactly the selected members. Unknown names fail before
    /// analysis; duplicates are deduplicated; `--path` is ignored.
    /// Conflicts with `--workspace`, which already means "all members".
    #[arg(
        short = 'p',
        long = "package",
        value_name = "NAME",
        conflicts_with = "workspace"
    )]
    package: Vec<String>,

    /// Glob patterns for files to skip, relative to `--path`, or in workspace
    /// mode to each member's directory or the workspace root.
    /// Use `**` to cross directory boundaries. May be repeated.
    /// Appends to the default exclusions (see `--no-default-excludes`).
    /// A glob that excludes no file gets a warning.
    ///
    /// Examples: `--exclude 'src/generated/**'`  `--exclude '**/build.rs'`
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Disable the built-in default exclusions (`tests/**`, `benches/**`,
    /// `examples/**`), analyzing those directories like any other source.
    /// Overrides the `default-excludes` config key.
    #[arg(long)]
    no_default_excludes: bool,

    /// Analyze every file afresh: neither read nor write the complexity
    /// cache in `<target>/cargo-crap/`. Duplicate-triage verdicts stay
    /// cached. The `cache = false` config key does the same for a project.
    #[arg(long)]
    no_cache: bool,

    /// CRAP score above which a function is considered "crappy".
    /// Falls back to `.cargo-crap.toml` → built-in default (30).
    #[arg(long)]
    threshold: Option<f64>,

    /// Only print functions with a CRAP score at or above this cutoff.
    /// Also shows every row of the human table, which otherwise lists only
    /// the 10 worst functions below the threshold.
    #[arg(long, value_name = "SCORE")]
    min: Option<f64>,

    /// Limit the report to the top N crappiest functions. Also shows every
    /// row of the human table, which otherwise lists only the 10 worst
    /// functions below the threshold.
    #[arg(long, value_name = "N")]
    top: Option<usize>,

    /// Clear `--top` and `--min`, from the flags and from `.cargo-crap.toml`.
    /// Meant for a `--format json` baseline, which has to hold every
    /// function: a function missing from it reads as new, so a rise in its
    /// score is never compared. The human table keeps its cap.
    #[arg(long)]
    all_rows: bool,

    /// How to handle functions with complexity data but no coverage data.
    /// Falls back to `.cargo-crap.toml` → built-in default (pessimistic).
    #[arg(long, value_enum)]
    missing: Option<MissingPolicy>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,

    /// Print only aggregate statistics (total analyzed, crappy count, worst
    /// offender) instead of the per-function table. Compatible with all `--format` values
    /// except `json` and `github` (which are unaffected).
    #[arg(long)]
    summary: bool,

    /// Exit with a non-zero status if any function's CRAP score exceeds
    /// `--threshold`. Useful in CI.
    #[arg(long)]
    fail_above: bool,

    /// Suppress functions matching these glob patterns. May be repeated.
    ///
    /// An entry containing `/` or `**` is treated as a path glob and matches
    /// the file in which the function is defined; otherwise it matches the
    /// function name. Path globs analyze the file but hide its functions
    /// from the report, unlike `--exclude`, which skips files at walk time.
    ///
    /// Examples: `--allow 'Foo::*'`  `--allow 'generated_*'`
    ///           `--allow 'src/generated/**'`  `--allow 'tests/**'`
    #[arg(long, value_name = "GLOB")]
    allow: Vec<String>,

    /// JSON baseline from a previous `--format json` run. When provided the
    /// report shows per-function deltas (regressions, improvements, new).
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// With --output and --baseline, write the full gated signal count to stdout
    /// as JSON. A null means no signal comparison was available.
    #[arg(long, requires_all = ["output", "baseline"])]
    print_signal_regressions: bool,

    /// Exit non-zero if any function's CRAP score increased since `--baseline`.
    /// Requires `--baseline`.
    #[arg(long)]
    fail_regression: bool,

    /// Show `Unchanged` rows in `--baseline` mode (human / markdown formats).
    /// By default only changed functions are listed. Requires `--baseline`.
    /// Falls back to `.cargo-crap.toml` → off.
    #[arg(long)]
    show_unchanged: bool,

    /// Final ordering of report entries. `crap` (default) sorts by CRAP score
    /// descending; `file` sorts by `(file, function, line)` ascending, which is
    /// stable across score changes and suits committed JSON baselines. `--top` always
    /// selects the N highest-CRAP functions first, then `--sort` reorders them.
    /// Falls back to `.cargo-crap.toml` → `crap`.
    #[arg(long, value_enum)]
    sort: Option<SortArg>,

    /// Write output to FILE instead of stdout. Useful for saving a JSON
    /// baseline: `--format json --output baseline.json`.
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Maximum number of threads used for parallel source-file analysis.
    /// Defaults to rayon's host-sized pool. Useful in memory-constrained
    /// CI/Docker environments. Falls back to `.cargo-crap.toml` → host default.
    #[arg(long, value_name = "N")]
    jobs: Option<usize>,

    /// Tolerance used by the regression detector. CRAP-score deltas with
    /// absolute value at or below this count as `Unchanged` rather than
    /// `Regressed` / `Improved`. Falls back to `.cargo-crap.toml` →
    /// built-in default (0.01).
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    epsilon: Option<f64>,

    /// Base URL of the source-hosting repo (e.g. `https://github.com/owner/repo`).
    /// Combined with `--commit-ref`, makes Function and Location cells in
    /// `pr-comment` / `markdown` output clickable links to GitHub source.
    /// Defaults from `GITHUB_SERVER_URL` + `GITHUB_REPOSITORY` when those env
    /// vars are set (e.g. inside GitHub Actions).
    #[arg(long, value_name = "URL")]
    repo_url: Option<String>,

    /// Commit SHA or branch name to deep-link into. Defaults from `GITHUB_SHA`
    /// when set. Has no effect unless `--repo-url` is also set.
    #[arg(long, value_name = "REF")]
    commit_ref: Option<String>,

    /// Also report candidate duplicate functions: functions whose normalized
    /// structure is close enough to be worth a look. Off by default, since it
    /// is a second analysis over the same AST and costs time on a large tree.
    #[arg(long)]
    duplicates: bool,

    /// Similarity at or above which a pair is reported, in `0.0..=1.0`
    /// (default 0.82). Named apart from `--threshold`, which is the CRAP
    /// score threshold and has meant that since v0.1.
    #[arg(long, value_name = "SCORE")]
    dup_threshold: Option<f64>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum MissingPolicy {
    Pessimistic,
    Optimistic,
    Skip,
}

impl From<MissingPolicy> for MissingCoveragePolicy {
    fn from(p: MissingPolicy) -> Self {
        match p {
            MissingPolicy::Pessimistic => Self::Pessimistic,
            MissingPolicy::Optimistic => Self::Optimistic,
            MissingPolicy::Skip => Self::Skip,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum SortArg {
    Crap,
    File,
}

impl From<SortArg> for SortOrder {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Crap => Self::Crap,
            SortArg::File => Self::File,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum FormatArg {
    Human,
    Json,
    /// GitHub Actions workflow commands: one `::warning` per crappy function.
    Github,
    /// GitHub-Flavored Markdown table, exhaustive, with no row caps. Use this
    /// for archived artifacts and docs pages.
    Markdown,
    /// Opinionated PR-comment markdown: hides unchanged rows, caps each
    /// section, collapses non-critical info into `<details>` blocks. Designed
    /// for the GitHub PR comment use case where readability beats completeness.
    PrComment,
    /// SARIF 2.1.0 JSON for upload to GitHub Code Scanning, VS Code, and
    /// other static-analysis tools. Each crappy function becomes one
    /// warning-level result. Incompatible with `--baseline`.
    Sarif,
    /// Shields.io endpoint-badge JSON counting the functions above `--threshold`.
    /// Serve the file at a stable URL and embed it in a README via
    /// `https://img.shields.io/endpoint?url=…`. `--baseline` is ignored.
    Shields,
}

impl From<FormatArg> for Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Human => Self::Human,
            FormatArg::Json => Self::Json,
            FormatArg::Github => Self::GitHub,
            FormatArg::Markdown => Self::Markdown,
            FormatArg::PrComment => Self::PrComment,
            FormatArg::Sarif => Self::Sarif,
            FormatArg::Shields => Self::Shields,
        }
    }
}

/// Built-in default exclude globs (spec 14): the standard Cargo target
/// directories that rarely benefit from CRAP analysis. Globs are matched
/// relative to each analyzed root (each member root in `--workspace` mode),
/// so only the top-level directories match — `src/tests/` is unaffected.
/// Replaced wholesale by the `default-excludes` config key; emptied by
/// `--no-default-excludes`.
const DEFAULT_EXCLUDES: &[&str] = &["tests/**", "benches/**", "examples/**"];

/// A run's exclude globs, kept apart by how they match (spec 40): the
/// defaults only ever below each walked root, the user's also below the
/// workspace root in workspace mode.
#[derive(Debug, Clone, Default)]
struct Excludes {
    defaults: Vec<String>,
    /// Config `exclude`, then `--exclude`.
    user: Vec<String>,
    /// Where the `--exclude` globs start in `user`: the only ones a no-match
    /// warning names (spec 40).
    cli_start: usize,
}

/// Assemble the effective excludes (spec 14 precedence): the default list
/// (built-in, or replaced by `default-excludes`, or emptied by
/// `--no-default-excludes`), and the config and CLI `exclude` globs, which
/// always add to it and never replace it.
fn effective_excludes(
    no_default_excludes: bool,
    config_default_excludes: Option<Vec<String>>,
    config_exclude: Vec<String>,
    cli_exclude: Vec<String>,
) -> Excludes {
    let defaults = if no_default_excludes {
        Vec::new()
    } else {
        config_default_excludes
            .unwrap_or_else(|| DEFAULT_EXCLUDES.iter().map(ToString::to_string).collect())
    };
    let cli_start = config_exclude.len();
    let mut user = config_exclude;
    user.extend(cli_exclude);
    Excludes {
        defaults,
        user,
        cli_start,
    }
}

/// The `--exclude` globs that `matched` (one flag per `excludes.user`
/// glob) says excluded no file in the run.
fn unmatched_cli_excludes<'a>(
    excludes: &'a Excludes,
    matched: &[bool],
) -> Vec<&'a str> {
    debug_assert_eq!(excludes.user.len(), matched.len());
    let mut unmatched: Vec<&str> = Vec::new();
    for (glob, _) in excludes
        .user
        .iter()
        .zip(matched)
        .skip(excludes.cli_start)
        .filter(|(_, hit)| !**hit)
    {
        if !unmatched.contains(&glob.as_str()) {
            unmatched.push(glob);
        }
    }
    unmatched
}

/// The workspace root in `--workspace` and `-p` mode, as every walked root
/// records it; `None` in single-path mode.
fn workspace_root(roots: &[ScanRoot]) -> Option<&Path> {
    roots.iter().find_map(|r| r.workspace_root.as_deref())
}

/// One stderr warning per `--exclude` glob the walk never matched (spec 40).
fn warn_unmatched_excludes(
    excludes: &Excludes,
    matched: &[bool],
    roots: &[ScanRoot],
) {
    let workspace = workspace_root(roots).is_some();
    for glob in unmatched_cli_excludes(excludes, matched) {
        eprintln!("{}", no_match_warning(glob, workspace));
    }
}

fn no_match_warning(
    glob: &str,
    workspace: bool,
) -> String {
    let base = if workspace {
        "each member's directory or the workspace root"
    } else {
        "`--path`"
    };
    format!("warning: --exclude '{glob}' excluded no file; globs are relative to {base}")
}

/// Strip the leading `crap` token inserted by cargo when invoked as a subcommand.
///
/// `cargo crap [args]` → cargo calls `cargo-crap crap [args]`. We strip that
/// extra token so clap sees `cargo-crap [args]` as it expects.
fn strip_cargo_subcommand(mut args: Vec<String>) -> Vec<String> {
    if args.get(1).map(String::as_str) == Some("crap") {
        args.remove(1);
    }
    args
}

/// One Cargo workspace member: package name + the directory containing its
/// `Cargo.toml`.
#[derive(Debug, Clone)]
struct WorkspaceMember {
    name: String,
    dir: PathBuf,
}

/// One tree that was walked, with the excludes that were in force for it.
///
/// Recorded rather than recomputed so the duplicate pass walks exactly what
/// the complexity pass walked. Deriving the roots twice is how `--workspace`
/// came to score member roots while scanning only `--path` for duplicates.
#[derive(Debug, Clone)]
struct ScanRoot {
    dir: PathBuf,
    /// Shared with the baseline scope after the walk has captured its match counts.
    matcher: Arc<complexity::ExcludeMatcher>,
    /// `None` outside workspace mode, where `dir` is the only base.
    workspace_root: Option<PathBuf>,
}

impl ScanRoot {
    /// `dir` under the run's `excludes`, plus `nested` (other members' roots
    /// below `dir`), which match below `dir` only, like the defaults.
    fn new(
        dir: PathBuf,
        excludes: &Excludes,
        nested: Vec<String>,
        workspace_root: Option<PathBuf>,
    ) -> Result<Self> {
        let mut member_only = excludes.defaults.clone();
        member_only.extend(nested);
        let context = format!("analyzing {}", dir.display());
        let matcher = complexity::ExcludeMatcher::new(
            &member_only,
            &excludes.user,
            workspace_root.as_deref(),
        )
        .context(context);
        Self::from_matcher(dir, matcher, workspace_root)
    }

    fn from_matcher(
        dir: PathBuf,
        matcher: Result<complexity::ExcludeMatcher>,
        workspace_root: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            dir,
            matcher: Arc::new(matcher?),
            workspace_root,
        })
    }
}

/// What [`analyze_sources`] hands back to `main`.
struct AnalyzedSources {
    fns: Vec<complexity::FunctionComplexity>,
    /// Every root walked, in order, with its effective excludes.
    roots: Vec<ScanRoot>,
    /// One flag per user exclude glob: whether the complexity walk excluded
    /// any file with it, in any root.
    user_globs_matched: Vec<bool>,
    /// The analyzed workspace members: all of them under `--workspace`, the
    /// selected subset under `-p/--package`, empty for single-root runs.
    members: Vec<WorkspaceMember>,
    /// Baseline scope attribution for `-p` runs (spec 25). `None` unless
    /// `-p` narrowed the analysis to a subset of the discovered members.
    member_scope: Option<MemberScope>,
    /// The target directory every cache lives under (spec 10), whether or
    /// not the complexity cache was used: the triage cache uses it too.
    target: Option<PathBuf>,
}

/// Walk source trees and discover Cargo workspace members in one pass.
///
/// Caps rayon's global thread pool to `jobs` first, so the parallel
/// `analyze_tree` walk respects user-set bounds. In `--workspace` mode all
/// members discovered via `cargo metadata` are walked; with `-p/--package`
/// only the selected members are (spec 25). Either way, a member walk never
/// descends into another member's nested root — each file is analyzed once,
/// by the member that owns it.
fn analyze_sources(
    workspace: bool,
    packages: &[String],
    path: &std::path::Path,
    excludes: &Excludes,
    knobs: &AnalysisKnobs,
) -> Result<AnalyzedSources> {
    if let Some(n) = knobs.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .with_context(|| format!("configuring rayon thread pool to {n} threads"))?;
    }
    if !workspace && packages.is_empty() {
        let root = ScanRoot::new(path.to_path_buf(), excludes, Vec::new(), None)?;
        let (fns, user_globs_matched, target) = analyze_path(&root, knobs);
        return Ok(AnalyzedSources {
            fns,
            roots: vec![root],
            user_globs_matched,
            members: Vec::new(),
            member_scope: None,
            target,
        });
    }
    analyze_workspace_members(packages, excludes, knobs)
}

/// Analyze one `--path` root, through the complexity cache when there is a
/// target directory to keep it in (spec 10). Without one, or when the
/// running executable cannot be identified, the walk is uncached.
fn analyze_path(
    root: &ScanRoot,
    knobs: &AnalysisKnobs,
) -> (
    Vec<complexity::FunctionComplexity>,
    Vec<bool>,
    Option<PathBuf>,
) {
    let path = &root.dir;
    let config_dir = knobs.config_dir.as_deref();
    let target = target_dir(None, path, config_dir, &knobs.cwd, |name| {
        std::env::var(name).ok()
    });
    let session = target
        .as_deref()
        .filter(|_| knobs.cache)
        .and_then(|target| Session::open(target, knobs.try_weight));
    let mut records = Vec::new();
    let (fns, matched) = analyze_root(root, knobs.try_weight, session.as_ref(), &mut records);
    if let Some(session) = session {
        session.save(std::slice::from_ref(path), records);
    }
    (fns, matched, target)
}

/// Walk one root, through `session`'s cache when there is one, adding the
/// records to store to `records`. Returns the functions and, per user glob,
/// whether it excluded any file under this root.
fn analyze_root(
    root: &ScanRoot,
    try_weight: f64,
    session: Option<&Session>,
    records: &mut Vec<FileRecord>,
) -> (Vec<complexity::FunctionComplexity>, Vec<bool>) {
    let excludes = &root.matcher;
    let fns = session.map_or_else(
        || complexity::analyze_tree_matching(&root.dir, excludes, try_weight),
        |session| {
            let analysis = complexity::analyze_tree_cached(&root.dir, excludes, session.cache());
            records.extend(analysis.records);
            analysis.functions
        },
    );
    (fns, excludes.user_globs_matched())
}

/// Mark in `all` every user glob `root` says matched under one more root.
fn add_matches(
    all: &mut [bool],
    root: &[bool],
) {
    for (any, &hit) in all.iter_mut().zip(root) {
        *any |= hit;
    }
}

/// The workspace side of [`analyze_sources`]: discover members, narrow to
/// the `-p` selection (all members when empty, i.e. `--workspace`), and
/// walk each selected member's root.
fn analyze_workspace_members(
    packages: &[String],
    excludes: &Excludes,
    knobs: &AnalysisKnobs,
) -> Result<AnalyzedSources> {
    let Workspace {
        root: workspace_root,
        target,
        members: discovered,
    } = workspace_members()?;
    let members = if packages.is_empty() {
        discovered.clone()
    } else {
        select_members(&discovered, packages)?
    };
    // One cache for every member, in the directory cargo builds into
    // (spec 10): opened once, saved once, so members never evict each other.
    let session = knobs
        .cache
        .then(|| Session::open(&target, knobs.try_weight))
        .flatten();
    let cache = session.as_ref();
    let mut records = Vec::new();
    let mut fns = Vec::new();
    let mut roots = Vec::new();
    let mut user_globs_matched = vec![false; excludes.user.len()];
    for m in &members {
        // Nested exclusion is computed against every discovered member, not
        // just the selected ones: an unselected nested member's files must
        // not leak into its parent's walk either.
        let root = ScanRoot::new(
            m.dir.clone(),
            excludes,
            nested_member_excludes(&m.dir, &discovered),
            Some(workspace_root.clone()),
        )?;
        let (member_fns, matched) = analyze_root(&root, knobs.try_weight, cache, &mut records);
        fns.extend(member_fns);
        add_matches(&mut user_globs_matched, &matched);
        roots.push(root);
    }
    save_members(session, &discovered, &members, records);
    let member_scope =
        (!packages.is_empty()).then(|| MemberScope::new(&workspace_root, &discovered, &members));
    Ok(AnalyzedSources {
        fns,
        roots,
        user_globs_matched,
        members,
        member_scope,
        target: Some(target),
    })
}

/// Save a workspace run's cache, if it has one: the selected members' directories were
/// walked; an unselected member nested inside one of them was not, so its
/// entries are kept.
fn save_members(
    session: Option<Session>,
    discovered: &[WorkspaceMember],
    selected: &[WorkspaceMember],
    records: Vec<FileRecord>,
) {
    let Some(session) = session else {
        return;
    };
    let walked: Vec<PathBuf> = selected.iter().map(|m| m.dir.clone()).collect();
    let kept: Vec<PathBuf> = discovered
        .iter()
        .filter(|m| !walked.contains(&m.dir))
        .map(|m| m.dir.clone())
        .collect();
    session.save_keeping(&walked, &kept, records);
}

/// One discovered member as seen by [`MemberScope`]: its directory
/// (forward-slash normalized, for prefix tests against same-root baseline
/// paths), its workspace-relative components (for suffix-window tests
/// against cross-root baseline paths, spec 21), and whether it was selected.
struct ScopedMember {
    dir: PathBuf,
    rel: Vec<String>,
    selected: bool,
}

/// Baseline scope for `-p/--package` runs (spec 25): attributes each
/// baseline entry to the discovered member that owns it, so entries owned
/// by unselected members vanish before the delta instead of flooding
/// `removed` (or feeding the pass-2 name matcher phantom moves).
///
/// Attribution is two-tier:
/// 1. **Prefix** (same-root baselines, the common case): the deepest member
///    dir that is a component-prefix of the entry path wins — exact, so a
///    root-package member correctly claims files outside every child
///    member dir.
/// 2. **Component window** (cross-root baselines): the member whose
///    workspace-relative dir appears as the longest contiguous component
///    run in the path wins. Ties that include a selected member keep the
///    entry, and the root package (empty rel) never claims cross-root
///    paths — when in doubt, keep the entry (today's behaviour: it shows
///    in `removed`, which is noisy but never hides a pairing).
struct MemberScope {
    members: Vec<ScopedMember>,
}

impl MemberScope {
    fn new(
        workspace_root: &Path,
        discovered: &[WorkspaceMember],
        selected: &[WorkspaceMember],
    ) -> Self {
        let selected_names: std::collections::HashSet<&str> =
            selected.iter().map(|m| m.name.as_str()).collect();
        let members = discovered
            .iter()
            .map(|m| ScopedMember {
                dir: PathBuf::from(m.dir.to_string_lossy().replace('\\', "/")),
                rel: m
                    .dir
                    .strip_prefix(workspace_root)
                    .map(|rel| {
                        rel.to_string_lossy()
                            .replace('\\', "/")
                            .split('/')
                            .filter(|c| !c.is_empty())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                selected: selected_names.contains(m.name.as_str()),
            })
            .collect();
        Self { members }
    }

    /// True when `file` is owned by an unselected member — the entry must
    /// be dropped before the delta.
    fn is_out_of_scope(
        &self,
        file: &Path,
    ) -> bool {
        let normalized = PathBuf::from(file.to_string_lossy().replace('\\', "/"));

        // Tier 1: deepest member dir that prefixes the path (exact).
        if let Some(owner) = self
            .members
            .iter()
            .filter(|m| normalized.starts_with(&m.dir))
            .max_by_key(|m| m.dir.components().count())
        {
            return !owner.selected;
        }

        // Tier 2: longest workspace-relative component window (cross-root).
        let components: Vec<&str> = normalized
            .to_str()
            .map(|s| s.split('/').filter(|c| !c.is_empty()).collect())
            .unwrap_or_default();
        let best_len = self
            .members
            .iter()
            .filter(|m| components_contain(&components, &m.rel))
            .map(|m| m.rel.len())
            .max();
        let Some(best_len) = best_len else {
            return false;
        };
        !self
            .members
            .iter()
            .filter(|m| m.rel.len() == best_len && components_contain(&components, &m.rel))
            .any(|m| m.selected)
    }
}

/// True when `window` appears as a contiguous run inside `haystack`.
/// An empty window never matches — the root package's empty rel must not
/// claim cross-root paths (see [`MemberScope`]).
fn components_contain(
    haystack: &[&str],
    window: &[String],
) -> bool {
    !window.is_empty()
        && haystack
            .windows(window.len())
            .any(|w| w.iter().zip(window).all(|(a, b)| *a == b.as_str()))
}

/// Resolve `-p/--package` selections against the discovered members.
/// Unknown names abort before any analysis; duplicate selections collapse
/// (the filter over `discovered` yields each member at most once); the
/// discovery order is preserved.
fn select_members(
    discovered: &[WorkspaceMember],
    requested: &[String],
) -> Result<Vec<WorkspaceMember>> {
    let available: std::collections::HashSet<&str> =
        discovered.iter().map(|m| m.name.as_str()).collect();
    let mut unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|n| !available.contains(n))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        unknown.dedup();
        let mut names: Vec<&str> = discovered.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        bail!(
            "unknown package(s): {}; available workspace members: {}",
            unknown.join(", "),
            names.join(", "),
        );
    }
    let wanted: std::collections::HashSet<&str> = requested.iter().map(String::as_str).collect();
    Ok(discovered
        .iter()
        .filter(|m| wanted.contains(m.name.as_str()))
        .cloned()
        .collect())
}

/// Exclude globs for other members' roots nested beneath `root`, so a member
/// walk never descends into a nested member's files (spec 25). The nested
/// member owns them and is walked separately when itself selected.
fn nested_member_excludes(
    root: &std::path::Path,
    discovered: &[WorkspaceMember],
) -> Vec<String> {
    discovered
        .iter()
        .filter(|m| m.dir != root)
        .filter_map(|m| m.dir.strip_prefix(root).ok())
        .map(|rel| format!("{}/**", rel.to_string_lossy().replace('\\', "/")))
        .collect()
}

/// Assign a Cargo workspace member name to each entry by matching the entry's
/// file path against member root directories. When a path lies inside more
/// than one root (nested workspaces), the longest match wins so the deepest
/// crate claims the file. No-op when `members` is empty.
fn assign_crate_names(
    entries: &mut [cargo_crap::merge::CrapEntry],
    members: &[WorkspaceMember],
) {
    if members.is_empty() {
        return;
    }
    // Pre-sort roots by descending path length so the first containing match
    // is also the deepest.
    let mut sorted: Vec<&WorkspaceMember> = members.iter().collect();
    sorted.sort_by_key(|m| std::cmp::Reverse(m.dir.as_os_str().len()));

    for entry in entries.iter_mut() {
        for m in &sorted {
            if entry.file.starts_with(&m.dir) {
                entry.crate_name = Some(m.name.clone());
                break;
            }
        }
    }
}

struct CargoScope {
    /// Deepest root first, so a nested member owns its files.
    roots: Vec<(PathBuf, Arc<complexity::ExcludeMatcher>)>,
    members: Option<MemberScope>,
}

impl CargoScope {
    fn new(
        roots: &[ScanRoot],
        members: Option<MemberScope>,
    ) -> Self {
        let mut roots: Vec<_> = roots
            .iter()
            .map(|root| (root.dir.clone(), Arc::clone(&root.matcher)))
            .collect();
        roots.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.as_os_str().len()));
        Self { roots, members }
    }

    /// True when the walk would have excluded `file`, normalized to forward
    /// slashes (cross-platform baselines): matched by the deepest analyzed
    /// root's matcher. A path under no root is matched whole.
    fn is_excluded(
        &self,
        file: &Path,
    ) -> bool {
        let normalized = PathBuf::from(file.to_string_lossy().replace('\\', "/"));
        let owner = self
            .roots
            .iter()
            .find(|(dir, _)| normalized.starts_with(dir))
            .map(|(dir, excludes)| (dir.as_path(), excludes))
            .or_else(|| {
                self.roots
                    .last()
                    .map(|(_, excludes)| (Path::new(""), excludes))
            });
        owner.is_some_and(|(dir, excludes)| excludes.check(dir, &normalized).excluded)
    }
}

impl Scope for CargoScope {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        !self.is_excluded(file)
            && self
                .members
                .as_ref()
                .is_none_or(|scope| !scope.is_out_of_scope(file))
    }
}

/// Parse the LCOV file if one was provided, returning an empty map otherwise.
fn load_coverage(lcov: Option<&PathBuf>) -> Result<HashMap<PathBuf, FileCoverage>> {
    match lcov {
        Some(path) => coverage::parse_lcov(path)
            .with_context(|| format!("parsing LCOV file {}", path.display())),
        None => Ok(HashMap::new()),
    }
}

/// Turn ANSI colour on or off for this run, from the environment, whether
/// output goes to an `--output` file, and whether stdout is a terminal.
fn apply_color_policy(output_to_file: bool) {
    let env_set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    set_color_enabled(resolve_color(
        env_set("NO_COLOR"),
        env_set("FORCE_COLOR"),
        output_to_file,
        io::stdout().is_terminal(),
    ));
}

/// Decide whether the human/summary renderers may emit ANSI colour.
///
/// Precedence: `NO_COLOR` (non-empty) always disables; `FORCE_COLOR`
/// (non-empty) then forces colour even into files and pipes; otherwise
/// colour is on only when writing to stdout and stdout is a terminal —
/// never into an `--output` file. Pure so the whole truth table is
/// unit-testable (a real TTY is unavailable under CI).
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the bools are independent environment facts (env vars, --output, TTY); \
              bundling them into a struct would only rename the same four flags"
)]
fn resolve_color(
    no_color: bool,
    force_color: bool,
    output_is_file: bool,
    stdout_is_tty: bool,
) -> bool {
    if no_color {
        false
    } else if force_color {
        true
    } else {
        !output_is_file && stdout_is_tty
    }
}

/// Prepare the output destination: set the process-wide colour policy for
/// it, then open it, a file when `--output` is given and stdout otherwise.
fn prepare_output(path: Option<&PathBuf>) -> Result<Box<dyn Write>> {
    apply_color_policy(path.is_some());
    Ok(match path {
        Some(p) => {
            Box::new(BufWriter::new(File::create(p).with_context(|| {
                format!("creating output file {}", p.display())
            })?))
        },
        None => Box::new(io::stdout()),
    })
}

/// Create a stderr spinner for the given message. Automatically suppressed
/// when stderr is not a TTY (CI, pipes).
fn spinner(msg: &'static str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""]),
    );
    pb.set_message(msg);
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// What `cargo metadata` says about the workspace.
#[derive(Debug)]
struct Workspace {
    /// The workspace root (spec 25 anchors members' relative dirs on it).
    root: PathBuf,
    /// The directory cargo builds into, every variable and
    /// `.cargo/config.toml` applied: where the caches live (spec 10).
    target: PathBuf,
    /// Every package in the workspace.
    members: Vec<WorkspaceMember>,
}

/// Discover the workspace via `cargo metadata`: its root, the directory
/// cargo builds into, and one [`WorkspaceMember`] per member crate (name +
/// the directory containing its `Cargo.toml`). The members are used both to
/// walk source trees and to assign a `crate` field to each `CrapEntry` for
/// per-crate rollup.
fn workspace_members() -> Result<Workspace> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .context("running `cargo metadata`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`cargo metadata` failed: {stderr}");
    }
    parse_workspace_metadata(&output.stdout)
}

/// Parse `cargo metadata` JSON into the workspace root (spec 25 — it
/// anchors the members' workspace-relative dirs for [`MemberScope`]), its
/// target directory (spec 10) and the member list. Split from
/// [`workspace_members`] so the parsing is testable without spawning cargo.
fn parse_workspace_metadata(stdout: &[u8]) -> Result<Workspace> {
    let meta: serde_json::Value =
        serde_json::from_slice(stdout).context("parsing `cargo metadata` output")?;

    let (workspace_root, target) = workspace_paths(&meta)?;

    let members: Vec<WorkspaceMember> = meta["packages"]
        .as_array()
        .context("`cargo metadata` output missing `packages`")?
        .iter()
        .filter_map(|pkg| {
            let name = pkg["name"].as_str()?.to_string();
            let dir = pkg["manifest_path"]
                .as_str()
                .and_then(|p| PathBuf::from(p).parent().map(std::path::Path::to_path_buf))?;
            Some(WorkspaceMember { name, dir })
        })
        .collect();

    if members.is_empty() {
        bail!("`cargo metadata` returned no packages");
    }
    Ok(Workspace {
        root: workspace_root,
        target,
        members,
    })
}

/// The workspace root and target directory a `cargo metadata` document
/// names; either one missing is an error.
fn workspace_paths(meta: &serde_json::Value) -> Result<(PathBuf, PathBuf)> {
    let path = |key: &str| {
        meta[key]
            .as_str()
            .map(PathBuf::from)
            .with_context(|| format!("`cargo metadata` output missing `{key}`"))
    };
    Ok((path("workspace_root")?, path("target_directory")?))
}

/// Lead line of the scope-mismatch warning, picked by overlap severity
/// (spec 24): zero matches and sub-50% overlap get an explicit
/// different-scopes verdict; smaller mismatches keep the neutral wording.
fn scope_warning_lead(diag: &ScopeDiagnostics) -> String {
    if diag.matched_files == 0 {
        format!(
            "warning: no overlap between the analyzed sources and the LCOV report \
             ({} analyzed, {} in LCOV, 0 matched) — the analyzed tree and the \
             coverage run describe different scopes",
            diag.analyzed_files, diag.lcov_files,
        )
    } else if diag.matched_files * 2 < diag.analyzed_files {
        format!(
            "warning: only {} of {} analyzed files match the LCOV report — the \
             analyzed tree and the coverage run likely describe different scopes",
            diag.matched_files, diag.analyzed_files,
        )
    } else {
        format!(
            "warning: source/LCOV scope mismatch ({} analyzed, {} in LCOV, {} matched) \
             — verify your --lcov path or coverage tool configuration",
            diag.analyzed_files, diag.lcov_files, diag.matched_files,
        )
    }
}

/// Detail lines for one stray side: `count` + `label` header, then the
/// bounded examples, then a `... and N more` tail when the cap truncated.
fn stray_lines(
    label: &str,
    strays: &cargo_crap::merge::StrayFiles,
) -> Vec<String> {
    if strays.count == 0 {
        return Vec::new();
    }
    let mut lines = vec![format!("  {} {label}:", strays.count)];
    lines.extend(
        strays
            .examples
            .iter()
            .map(|f| format!("    {}", f.display())),
    );
    if strays.count > strays.examples.len() {
        lines.push(format!(
            "    ... and {} more",
            strays.count - strays.examples.len()
        ));
    }
    lines
}

/// Emit the scope-mismatch warning (spec 24) to stderr, ahead of the report.
///
/// Silent when there are no diagnostics (no `--lcov`) or when both stray
/// sides are empty (scopes match exactly).
fn warn_scope_mismatch(diag: Option<&ScopeDiagnostics>) {
    let Some(d) = diag else { return };
    if d.source_only.count == 0 && d.lcov_only.count == 0 {
        return;
    }
    eprintln!("{}", scope_warning_lead(d));
    let details = stray_lines(
        "analyzed source file(s) had no matching entry in the LCOV report",
        &d.source_only,
    )
    .into_iter()
    .chain(stray_lines(
        "LCOV file(s) matched no analyzed source file",
        &d.lcov_only,
    ));
    for line in details {
        eprintln!("{line}");
    }
}

/// Resolve a boolean flag against config: the CLI switch wins when set,
/// otherwise the config value, defaulting to false. Keeps the `||` decision
/// points out of `main` so it stays under the CC budget.
fn resolve_bool(
    cli_flag: bool,
    config_value: Option<bool>,
) -> bool {
    cli_flag || config_value.unwrap_or(false)
}

/// Resolve the missing-coverage policy: the CLI flag wins when given, then
/// the config value, then pessimistic, the default a CI gate wants.
fn resolve_missing(
    cli_flag: Option<MissingPolicy>,
    config_value: Option<MissingCoveragePolicy>,
) -> MissingCoveragePolicy {
    cli_flag
        .map(Into::into)
        .or(config_value)
        .unwrap_or(MissingCoveragePolicy::Pessimistic)
}

/// The `(min, top)` display limits: the flag over the config, or neither
/// with `--all-rows`.
fn display_limits(
    all_rows: bool,
    cli: (Option<f64>, Option<usize>),
    config: (Option<f64>, Option<usize>),
) -> (Option<f64>, Option<usize>) {
    if all_rows {
        (None, None)
    } else {
        (cli.0.or(config.0), cli.1.or(config.1))
    }
}

/// Bail when `flag` is set but `--baseline` is absent. Keeps the per-flag
/// `&& baseline.is_none()` checks out of [`validate_args`].
fn require_baseline(
    flag_set: bool,
    baseline_present: bool,
    flag: &str,
) -> Result<()> {
    if flag_set && !baseline_present {
        bail!("{flag} requires --baseline");
    }
    Ok(())
}

/// True when the run never reads `--path`: `--workspace` and `-p/--package`
/// both derive their roots from `cargo metadata` instead, so a stale or
/// missing `--path` must not fail validation.
fn path_is_ignored(cli: &Cli) -> bool {
    cli.workspace || !cli.package.is_empty()
}

/// Validate knobs that can arrive from either the CLI or the config file.
///
/// [`validate_args`] only sees raw CLI values, so a config-sourced value
/// must be re-checked after the merge — otherwise `.cargo-crap.toml`
/// smuggles in values the equivalent flag would reject (a negative epsilon
/// silently classifies every unchanged function as `Regressed`; the config
/// docs promise both bounds).
fn validate_merged_values(
    epsilon: f64,
    jobs: Option<usize>,
    dup: &DupSettings,
    try_weight: f64,
) -> Result<()> {
    if epsilon < 0.0 {
        bail!("invalid epsilon value (--epsilon or config): must be non-negative");
    }
    if matches!(jobs, Some(0)) {
        bail!("invalid jobs value (--jobs or config): must be a positive integer");
    }
    // The merged value, not just the flag: a threshold set in
    // `.cargo-crap.toml` reaches the comparison the same way, and an
    // unchecked one silently reports everything or nothing. `!(..)` rather
    // than `<`/`>` so that NaN — which TOML can express — is rejected too.
    if !(0.0..=1.0).contains(&dup.threshold) {
        bail!(
            "invalid duplicates threshold (--dup-threshold or config): \
             must be between 0.0 and 1.0"
        );
    }
    validate_triage_floor(dup.triage_floor).and_then(|()| validate_try_weight(try_weight))
}

/// Reject a triage confidence floor outside `0.0..=1.0`, NaN included,
/// which TOML can spell. Config-only, so the message names no flag.
fn validate_triage_floor(floor: f64) -> Result<()> {
    if !(0.0..=1.0).contains(&floor) {
        bail!(
            "invalid duplicates.triage.confidence-floor (config): \
             must be between 0.0 and 1.0"
        );
    }
    Ok(())
}

/// Reject a `?` weight that is negative, NaN, infinite or above the cap.
/// TOML can spell all of them, and each would make every CC and CRAP score
/// it touches meaningless (a huge finite weight overflows them to inf/NaN).
/// `!contains` rather than `<`/`>` so NaN is rejected too. Config-only, so
/// the message names no flag.
fn validate_try_weight(try_weight: f64) -> Result<()> {
    use cargo_crap::config::MAX_TRY_WEIGHT;
    if !(0.0..=MAX_TRY_WEIGHT).contains(&try_weight) {
        bail!(
            "invalid try-weight value (config): must be a non-negative number \
             no greater than {MAX_TRY_WEIGHT}"
        );
    }
    Ok(())
}

/// Validate argument combinations that clap cannot express as declarative rules.
fn validate_args(cli: &Cli) -> Result<()> {
    if !path_is_ignored(cli) && !cli.path.exists() {
        bail!("path does not exist: {}", cli.path.display());
    }
    let has_baseline = cli.baseline.is_some();
    require_baseline(cli.fail_regression, has_baseline, "--fail-regression")?;
    require_baseline(cli.show_unchanged, has_baseline, "--show-unchanged")?;
    if matches!(cli.jobs, Some(0)) {
        bail!("invalid --jobs value: must be a positive integer");
    }
    if let Some(eps) = cli.epsilon
        && eps < 0.0
    {
        bail!("invalid --epsilon value: must be non-negative");
    }
    if let Some(t) = cli.dup_threshold
        && !(0.0..=1.0).contains(&t)
    {
        bail!("invalid --dup-threshold value: must be between 0.0 and 1.0");
    }
    Ok(())
}

fn load_report_baseline(
    baseline: Option<&PathBuf>,
    workspace_root: Option<&Path>,
    try_weight: f64,
    signals_enabled: bool,
) -> Result<Option<Baseline>> {
    let Some(baseline_path) = baseline else {
        return Ok(None);
    };
    let baseline = load_baseline_with_weight(baseline_path, &cargo_crap::tool::CARGO_CRAP)?;
    warn_on_try_weight_mismatch(baseline.try_weight, try_weight);
    warn_on_missing_signals(&baseline.entries, signals_enabled);
    let mut baseline = baseline;
    resolve_relative_paths(&mut baseline, workspace_root);
    Ok(Some(baseline))
}

/// In workspace mode, read every relative path in `baseline` as relative to
/// the workspace root, as a workspace-mode report writes it (spec 41), so the
/// filter, the member scope and the delta compare absolute with absolute.
fn resolve_relative_paths(
    baseline: &mut Baseline,
    workspace_root: Option<&Path>,
) {
    let Some(root) = workspace_root else {
        return;
    };
    // Normalize separators so paths recorded on Windows resolve. Joining an absolute
    // path preserves it, so both path forms can use the same operation.
    let resolve = |file: &mut PathBuf| *file = root.join(file.to_string_lossy().replace('\\', "/"));
    for entry in &mut baseline.entries {
        resolve(&mut entry.file);
    }
    for member in baseline.duplicate_groups.iter_mut().flatten().flatten() {
        resolve(&mut member.file);
    }
}

fn warn_on_try_weight_mismatch(
    baseline: f64,
    current: f64,
) {
    if let Some(warning) = try_weight_mismatch_warning(baseline, current) {
        eprintln!("{warning}");
    }
}

/// The warning for a baseline scored under a different `?` weight than this
/// run, or `None` when the weights match.
#[expect(
    clippy::float_cmp,
    reason = "any difference in the weight rescored every function with a `?`; there is no tolerance to apply"
)]
fn try_weight_mismatch_warning(
    baseline: f64,
    current: f64,
) -> Option<String> {
    (baseline != current).then(|| {
        format!(
            "warning: the baseline was recorded with try-weight {baseline} and this run \
             uses {current} — deltas reflect the weight change, not code changes"
        )
    })
}

fn warn_on_missing_signals(
    entries: &[cargo_crap::merge::CrapEntry],
    signals_enabled: bool,
) {
    if let Some(warning) = missing_signals_warning(entries, signals_enabled) {
        eprintln!("{warning}");
    }
}

/// The warning for a run with signals enabled against a baseline recorded
/// without them (spec 43): no signal can be compared, so none is, and the
/// gate falls back to CRAP alone. `None` when there is nothing to say.
fn missing_signals_warning(
    entries: &[cargo_crap::merge::CrapEntry],
    signals_enabled: bool,
) -> Option<String> {
    let none_recorded = !entries.is_empty() && entries.iter().all(|e| e.signals.is_none());
    (signals_enabled && none_recorded).then(|| {
        "warning: the baseline has no signals, so no signal was compared — regenerate it \
         with [signals] enabled to compare them"
            .to_owned()
    })
}

/// Everything the duplicate pass needs, resolved before `cli` and `config`
/// are consumed by the rest of the run.
struct DupSettings {
    enabled: bool,
    threshold: f64,
    min_nodes: usize,
    /// Whether to ask the model about each reported pair.
    triage_enabled: bool,
    /// The API to ask, resolved from its configured id.
    triage_provider: &'static dyn Provider,
    /// The model to ask.
    triage_model: String,
    /// Confidence below which a triage verdict names no kind. Carried here,
    /// range-checked with the threshold, so the triage pass reads the value
    /// that was validated rather than re-resolving its own.
    triage_floor: f64,
    /// The most paid triage requests the run may send; `None` for no limit.
    triage_max_requests: Option<u32>,
}

impl DupSettings {
    /// CLI wins over config, exactly as `threshold` and `fail_above` do.
    /// `provider` is the already resolved triage provider, so an unset model
    /// follows it.
    fn resolve(
        cli: &Cli,
        config: &cargo_crap::config::Config,
        provider: &'static dyn Provider,
    ) -> Self {
        let triage = &config.duplicates.triage;
        Self {
            enabled: cli.duplicates || config.duplicates.enabled.unwrap_or(false),
            threshold: cli
                .dup_threshold
                .or(config.duplicates.threshold)
                .unwrap_or(cargo_crap::config::DEFAULT_DUP_THRESHOLD),
            min_nodes: config
                .duplicates
                .min_nodes
                .unwrap_or(cargo_crap::config::DEFAULT_DUP_MIN_NODES),
            triage_enabled: triage.enabled.unwrap_or(false),
            triage_provider: provider,
            triage_model: triage
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_owned()),
            triage_floor: triage
                .confidence_floor
                .unwrap_or(cargo_crap::config::DEFAULT_TRIAGE_CONFIDENCE_FLOOR),
            triage_max_requests: triage.max_requests,
        }
    }
}

/// The provider registered under `id`, or the default provider when no id
/// is configured. Runs in every build: the registry needs no client.
///
/// # Errors
///
/// When `id` names no registered provider. The message lists the ones that
/// are.
fn resolve_triage_provider(id: Option<&str>) -> Result<&'static dyn Provider> {
    use cargo_crap::duplicates::triage::provider::{by_id, ids};
    let Some(id) = id else {
        return Ok(default_provider());
    };
    by_id(id).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid duplicates.triage.provider (config): {id:?} is not a provider; \
             expected one of: {}",
            ids().collect::<Vec<_>>().join(", ")
        )
    })
}

/// Run the duplicate pass, when it was asked for.
///
/// Returns `None` — not an empty vector — when detection is off, so the JSON
/// envelope can tell "not asked" from "asked, found nothing". Does no work at
/// all in that case: the walk, the parse and the pairwise sweep are a second
/// analysis over the same tree, and a run that did not ask must not pay.
///
/// Walks the roots the complexity pass actually walked, so `--workspace` and
/// `-p` compare each member's own tree under that member's own excludes.
fn duplicate_pairs(
    settings: &DupSettings,
    roots: &[ScanRoot],
    format: Format,
) -> Option<Vec<DuplicatePair>> {
    if !settings.enabled {
        return None;
    }
    // Only four formats can carry the pairs. Paying for a quadratic sweep and
    // then discarding it silently is worse than saying so and skipping it.
    if !format.carries_duplicates() {
        eprintln!(
            "warning: --duplicates has no effect with this format; \
             use --format human, json, markdown or pr-comment"
        );
        return None;
    }
    let mut functions = Vec::new();
    for root in roots {
        functions.extend(duplicates::scan::scan_matching(&root.dir, &root.matcher));
    }
    // Small functions are dropped before comparison rather than filtered out
    // of the results: every pair of trivial accessors is a genuine structural
    // match, and reporting them buries the pairs worth reading.
    functions.retain(|f| f.node_count >= settings.min_nodes);
    functions.sort_by(|a, b| a.location.cmp(&b.location));
    let mut pairs = duplicates::compare::find_pairs(&functions, settings.threshold);
    duplicates::compare::sort_pairs(&mut pairs);
    Some(pairs)
}

/// The duplicate section's content: the pairs, their groups, and one triage
/// verdict per group when it ran.
struct DuplicateSection<'a> {
    pairs: Option<Vec<DuplicatePair>>,
    /// The pairs grouped into functions that all resemble each other.
    groups: Option<Vec<duplicates::group::DuplicateGroup>>,
    /// The groups measured against the baseline's duplicate record, on a
    /// delta run whose baseline has one.
    novelty: Option<duplicates::novelty::Comparison>,
    /// One verdict per group, in the groups' order, `None` for a group
    /// nobody asked about.
    triage: Option<Vec<Option<Assessment>>>,
    /// How many groups the triage budget left unasked.
    untriaged: usize,
    /// The display name of the provider asked, named when triage answered.
    provider: &'static str,
    /// The model asked.
    model: &'a str,
}

impl DuplicateSection<'_> {
    /// Who answered the triage: the provider and the model asked, when
    /// triage gave at least one verdict.
    fn triaged_by(&self) -> Option<TriagedBy<'_>> {
        let answered = |t: &&Vec<Option<Assessment>>| t.iter().any(Option::is_some);
        self.triage.as_ref().filter(answered).map(|_| TriagedBy {
            provider: self.provider,
            model: self.model,
        })
    }

    fn render_extras(
        &self,
        try_weight: f64,
    ) -> RenderExtras<'_> {
        RenderExtras {
            duplicate_row_limit: None,
            duplicate_context: None,
            duplicates: self.pairs.as_deref(),
            groups: self.groups.as_deref(),
            triage: self.triage.as_deref(),
            triaged_by: self.triaged_by(),
            novelty: self.novelty.as_ref(),
            untriaged: self.untriaged,
            try_weight,
        }
    }
}

/// Find the duplicate pairs and group them, measure the groups against the
/// baseline's duplicate record when there is one, then triage each group's
/// most similar pair when configured to: every group, or on a delta run only
/// the new and grown ones. Triage only ever sees groups that will be
/// reported, so a format that cannot carry duplicates never triggers a
/// request.
fn duplicate_section<'a>(
    settings: &'a DupSettings,
    roots: &[ScanRoot],
    format: Format,
    target: Option<&Path>,
    baseline: Option<&Baseline>,
) -> DuplicateSection<'a> {
    let pairs = duplicate_pairs(settings, roots, format);
    let groups = pairs
        .as_deref()
        .map(|pairs| duplicates::group::group_pairs(pairs, settings.threshold));
    warn_on_missing_record(groups.as_deref(), baseline);
    let novelty = novelty_against(groups.as_deref(), baseline);
    // One request per group asked about, about its most similar pair.
    let triage = groups.as_deref().and_then(|groups| {
        let asked = asked(novelty.as_ref(), groups.len());
        let representatives: Vec<DuplicatePair> = asked
            .iter()
            .map(|&n| groups[n].representative.clone())
            .collect();
        triage_assessments(settings, &representatives, target)
            .map(|(verdicts, skipped)| (spread(&asked, verdicts, groups.len()), skipped))
    });
    let (triage, untriaged) =
        triage.map_or((None, 0), |(verdicts, skipped)| (Some(verdicts), skipped));
    DuplicateSection {
        pairs,
        groups,
        novelty,
        triage,
        untriaged,
        provider: settings.triage_provider.display_name(),
        model: &settings.triage_model,
    }
}

/// Each answer held to the confidence floor, `None` where the budget asked
/// nothing, and how many the budget left unasked, with its one warning when
/// it left any.
#[cfg(feature = "triage")]
fn assessments(
    triaged: cargo_crap::duplicates::triage::Triaged,
    settings: &DupSettings,
) -> (Vec<Option<Assessment>>, usize) {
    let warning = settings.triage_max_requests.and_then(|budget| {
        cargo_crap::duplicates::triage::budget::warning(budget, triaged.skipped)
    });
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    let verdicts = triaged
        .verdicts
        .into_iter()
        .map(|verdict| verdict.map(|v| v.assessment(settings.triage_floor)))
        .collect();
    (verdicts, triaged.skipped)
}

/// The groups triage asks about, by index: every group, or on a delta run
/// with a duplicate record the ones it reports.
fn asked(
    novelty: Option<&duplicates::novelty::Comparison>,
    groups: usize,
) -> Vec<usize> {
    novelty.map_or_else(|| (0..groups).collect(), |c| c.reported().collect())
}

/// The `verdicts` for the `asked` groups, each put back at its group's
/// index among `groups`, with `None` for the groups nobody asked about.
fn spread(
    asked: &[usize],
    verdicts: Vec<Option<Assessment>>,
    groups: usize,
) -> Vec<Option<Assessment>> {
    debug_assert_eq!(asked.len(), verdicts.len(), "one answer per question");
    let mut spread = vec![None; groups];
    for (&n, verdict) in asked.iter().zip(verdicts) {
        spread[n] = verdict;
    }
    spread
}

/// The warning for a delta run that found duplicate groups against a
/// baseline recorded without `--duplicates`, which leaves nothing to tell
/// the new groups from the old.
fn missing_record_warning(
    groups: Option<&[duplicates::group::DuplicateGroup]>,
    baseline: Option<&Baseline>,
) -> Option<&'static str> {
    let found = groups.is_some_and(|g| !g.is_empty());
    (found && baseline.is_some_and(|b| b.duplicate_groups.is_none())).then_some(
        "warning: the baseline has no duplicate record, so the new duplicates cannot be \
         told from the old. Record the baseline with --duplicates to tell them apart.",
    )
}

/// Print [`missing_record_warning`] when it applies.
fn warn_on_missing_record(
    groups: Option<&[duplicates::group::DuplicateGroup]>,
    baseline: Option<&Baseline>,
) {
    if let Some(warning) = missing_record_warning(groups, baseline) {
        eprintln!("{warning}");
    }
}

/// The current groups measured against the baseline's duplicate record,
/// when the run found groups and the baseline recorded some.
fn novelty_against(
    groups: Option<&[duplicates::group::DuplicateGroup]>,
    baseline: Option<&Baseline>,
) -> Option<duplicates::novelty::Comparison> {
    let recorded = baseline?.duplicate_groups.as_deref()?;
    Some(duplicates::novelty::compare(groups?, recorded))
}

/// Ask the model about `pairs` when triage is enabled, and hold each verdict
/// to the confidence floor.
///
/// Triage is advisory, so nothing about it can fail the run: no key, no
/// network, an API error or an answer that does not decode each become one
/// warning naming the cause, and the duplicates are reported untriaged,
/// exactly as with triage off. A build without the `triage` feature has no
/// client: it says how to get one and reports the duplicates untriaged.
fn triage_assessments(
    settings: &DupSettings,
    pairs: &[DuplicatePair],
    target: Option<&Path>,
) -> Option<(Vec<Option<Assessment>>, usize)> {
    if !settings.triage_enabled {
        return None;
    }
    #[cfg(feature = "triage")]
    {
        use cargo_crap::duplicates::triage;
        let api_settings = triage::Settings {
            max_requests: settings.triage_max_requests,
            ..triage::Settings::from_env(settings.triage_provider, &settings.triage_model, target)
        };
        triage::run(pairs, &api_settings)
            .inspect_err(|e| {
                eprintln!(
                    "warning: duplicate triage skipped: {e}; the duplicates are reported untriaged"
                );
            })
            .ok()
            .map(|triaged| assessments(triaged, settings))
    }
    #[cfg(not(feature = "triage"))]
    {
        let _ = (pairs, target, settings.triage_max_requests);
        eprintln!(
            "warning: [duplicates.triage] is enabled, but this cargo-crap was built without \
             the `triage` feature; reinstall with `cargo install cargo-crap --features triage` \
             to use it"
        );
        None
    }
}

/// Resolve `--repo-url` / `--commit-ref` against the GitHub Actions env vars,
/// returning `Some(SourceLinks)` only when both pieces are present. Either
/// missing — or both missing — results in `None` (plain rendering, no links).
fn resolve_source_links(
    cli_repo_url: Option<String>,
    cli_commit_ref: Option<String>,
    root: &Path,
    workspace_root: Option<&Path>,
) -> Option<SourceLinks> {
    let repo_url = cli_repo_url.or_else(|| {
        let server = std::env::var("GITHUB_SERVER_URL").ok()?;
        let repo = std::env::var("GITHUB_REPOSITORY").ok()?;
        Some(format!(
            "{}/{}",
            server.trim_end_matches('/'),
            repo.trim_start_matches('/')
        ))
    })?;
    let commit_ref = cli_commit_ref.or_else(|| std::env::var("GITHUB_SHA").ok())?;
    let links = SourceLinks::new(repo_url, commit_ref).with_root(root.to_path_buf());
    Some(workspace_root.map_or(links.clone(), |workspace_root| {
        links.with_workspace_root(workspace_root.to_path_buf())
    }))
}

/// Parse argv, validate flag combinations, and load the optional
/// `.cargo-crap.toml` (defaults when absent — the tool works without one).
/// Also returns the project root: the directory the config file was found
/// in, or the working directory when there was none.
fn parse_and_load_config() -> Result<(Cli, cargo_crap::config::Config, ProjectDirs)> {
    let cli = Cli::parse_from(strip_cargo_subcommand(std::env::args().collect()));
    validate_args(&cli)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| cli.path.clone());
    let config = cargo_crap::config::load(&cwd)?;
    let config_dir =
        cargo_crap::config::find(&cwd).and_then(|file| file.parent().map(Path::to_path_buf));
    Ok((cli, config, ProjectDirs { config_dir, cwd }))
}

/// Whether the complexity cache is used: `--no-cache` or `cache = false`
/// turns it off, and it is on otherwise (spec 10).
fn cache_enabled(
    no_cache: bool,
    config: Option<bool>,
) -> bool {
    !no_cache && config.unwrap_or(true)
}

/// Where the project is, as far as configuration can tell.
struct ProjectDirs {
    /// The directory `.cargo-crap.toml` was found in, if it was.
    config_dir: Option<PathBuf>,
    /// The working directory, read once.
    cwd: PathBuf,
}

/// Exit-code contract (spec 23): 0 = analysis completed and no requested
/// gate tripped; 1 = analysis completed, report written, a gate tripped;
/// 2 = the run did not complete (usage, input, analysis, or output error —
/// clap's own usage exit is also 2).
fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:?}");
            ExitCode::from(2)
        },
    }
}

/// Everything [`run`] needs after parsing: the raw CLI/config plus the
/// merged-and-validated values that [`validate_args`] (CLI-only) cannot
/// check itself.
struct LoadedArgs {
    cli: Cli,
    config: cargo_crap::config::Config,
    /// Resolved and range-checked duplicate settings.
    dup: DupSettings,
    epsilon: f64,
    /// The knobs the analysis itself reads.
    knobs: AnalysisKnobs,
    /// Resolved and validated `[signals]` table (spec 43).
    signals: cargo_crap::signals::SignalSettings,
}

/// What [`analyze_sources`] needs besides the roots and excludes.
struct AnalysisKnobs {
    jobs: Option<usize>,
    /// Resolved and range-checked `?` weight.
    try_weight: f64,
    /// The directory `.cargo-crap.toml` was found in, if any: the last
    /// place a cache's target directory is looked for (spec 10).
    config_dir: Option<PathBuf>,
    /// The working directory a relative `--path` or target variable is
    /// taken against.
    cwd: PathBuf,
    /// Whether the complexity cache is used: off when `--no-cache` is given
    /// or the config says `cache = false`.
    cache: bool,
}

/// Parse argv, load config, resolve the triage provider and validate the
/// merged epsilon, jobs, similarity threshold, triage confidence floor and
/// try-weight, returning exactly what was validated so [`run`] cannot
/// consume a different (unchecked) merge of the same knobs.
fn parse_and_validate() -> Result<LoadedArgs> {
    let (cli, config, dirs) = parse_and_load_config()?;
    let epsilon = cli
        .epsilon
        .or(config.epsilon)
        .unwrap_or(cargo_crap::delta::DEFAULT_EPSILON);
    let jobs = cli.jobs.or(config.jobs);
    let try_weight = config
        .try_weight
        .unwrap_or(cargo_crap::config::DEFAULT_TRY_WEIGHT);
    // Resolved here rather than at the call site: the merged similarity
    // threshold has to be validated before anything is analyzed, and the
    // config half of it is invisible to `validate_args`. Resolving the
    // provider is validating it, so it shares the one `?` with the other
    // merged values.
    let (dup, signals) = resolve_triage_provider(config.duplicates.triage.provider.as_deref())
        .map(|provider| DupSettings::resolve(&cli, &config, provider))
        .and_then(|dup| validate_merged_values(epsilon, jobs, &dup, try_weight).map(|()| dup))
        .and_then(|dup| {
            cargo_crap::signals::SignalSettings::resolve(&config.signals).map(|s| (dup, s))
        })?;
    let cache = cache_enabled(cli.no_cache, config.cache);
    Ok(LoadedArgs {
        cli,
        config,
        dup,
        epsilon,
        knobs: AnalysisKnobs {
            jobs,
            try_weight,
            config_dir: dirs.config_dir,
            cwd: dirs.cwd,
            cache,
        },
        signals,
    })
}

/// The settings `run` reads with CLI values taking precedence and config
/// filling in what is missing.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one resolved CLI/config switch; not a struct-design smell"
)]
struct Settings {
    threshold: f64,
    min: Option<f64>,
    top: Option<usize>,
    missing_policy: MissingCoveragePolicy,
    fail_above: bool,
    fail_regression: bool,
    show_unchanged: bool,
    uncovered_hints: bool,
    sort_order: SortOrder,
}

fn resolve_settings(
    cli: &Cli,
    config: &cargo_crap::config::Config,
) -> Settings {
    let (min, top) = display_limits(cli.all_rows, (cli.min, cli.top), (config.min, config.top));
    Settings {
        threshold: cli
            .threshold
            .or(config.threshold)
            .unwrap_or(DEFAULT_THRESHOLD),
        min,
        top,
        missing_policy: resolve_missing(cli.missing, config.missing),
        fail_above: resolve_bool(cli.fail_above, config.fail_above),
        fail_regression: resolve_bool(cli.fail_regression, config.fail_regression),
        show_unchanged: resolve_bool(cli.show_unchanged, config.show_unchanged),
        // Config-only knob: deliberately no CLI flag, so there is no CLI side
        // to resolve against.
        uncovered_hints: config.uncovered_hints.unwrap_or(false),
        sort_order: cli.sort.map(Into::into).or(config.sort).unwrap_or_default(),
    }
}

fn run() -> Result<ExitCode> {
    let LoadedArgs {
        cli,
        config,
        dup,
        epsilon,
        knobs,
        signals,
    } = parse_and_validate()?;
    let settings = resolve_settings(&cli, &config);

    let effective_exclude = effective_excludes(
        cli.no_default_excludes,
        config.default_excludes,
        config.exclude,
        cli.exclude,
    );
    let mut effective_allow = config.allow;
    effective_allow.extend(cli.allow);

    // --- Analysis ---
    let pb = spinner("Analyzing source files…");
    let AnalyzedSources {
        fns,
        roots,
        user_globs_matched,
        members,
        member_scope,
        target,
    } = analyze_sources(
        cli.workspace,
        &cli.package,
        &cli.path,
        &effective_exclude,
        &knobs,
    )?;

    pb.set_message("Parsing coverage report…");
    pb.suspend(|| warn_unmatched_excludes(&effective_exclude, &user_globs_matched, &roots));
    let coverage = load_coverage(cli.lcov.as_ref())?;
    pb.finish_and_clear();

    // --- Merge + filters ---
    let mut merge_result = merge(fns, coverage, settings.missing_policy);
    warn_scope_mismatch(merge_result.diagnostics.as_ref());
    assign_crate_names(&mut merge_result.entries, &members);
    let allow = AllowSet::compile(&effective_allow)?;
    let workspace_root = workspace_root(&roots);
    let raw = load_report_baseline(
        cli.baseline.as_ref(),
        workspace_root,
        knobs.try_weight,
        signals.enabled,
    )?;
    let scope = CargoScope::new(&roots, member_scope);
    let analysis = run::Analysis {
        entries: merge_result.entries,
        diagnostics: merge_result.diagnostics,
        scope: &scope,
        coverage_kind: None,
        notices: &[],
    };
    let baseline = raw.map(|raw| BaselineFilter::new(analysis.scope, &allow).prepare(raw));

    let mut out = prepare_output(cli.output.as_ref())?;
    let links = resolve_source_links(cli.repo_url, cli.commit_ref, &knobs.cwd, workspace_root);
    let duplicates = duplicate_section(
        &dup,
        &roots,
        cli.format.into(),
        target.as_deref(),
        baseline.as_ref().map(PreparedBaseline::as_baseline),
    );
    let report = run::Settings {
        allow: &allow,
        min: settings.min,
        top: settings.top,
        sort: settings.sort_order,
        threshold: settings.threshold,
        epsilon,
        signals: &signals,
        fail_above: settings.fail_above,
        fail_regression: settings.fail_regression,
        format: cli.format.into(),
        summary: cli.summary,
        show_unchanged: settings.show_unchanged,
        uncovered_hints: settings.uncovered_hints,
        width: output_width(cli.output.is_none()),
        links: links.as_ref(),
        cwd: &knobs.cwd,
        workspace_root,
        pr_comment_marker: None,
        repo_prefix: None,
        fail_sections: false,
    };
    let outcome = run::run(
        &cargo_crap::tool::CARGO_CRAP,
        &report,
        analysis,
        baseline.as_ref(),
        &[],
        &duplicates.render_extras(knobs.try_weight),
        out.as_mut(),
    )?;
    if cli.print_signal_regressions {
        serde_json::to_writer(io::stdout().lock(), &outcome.signal_regressions)?;
    }
    Ok(outcome.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cargo_crap::duplicates::extract::Location;

    /// An entry with or without signals, for the spec 43 baseline checks.
    fn signal_entry(signals: Option<cargo_crap::signals::Signals>) -> cargo_crap::merge::CrapEntry {
        cargo_crap::merge::CrapEntry {
            file: PathBuf::from("src/lib.rs"),
            function: "f".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: None,
            crap: 2.0,
            crate_name: None,
            signals,
            fingerprint: None,
            signal_spans: cargo_crap::signals::Spans::default(),
            uncovered: Vec::new(),
        }
    }

    #[test]
    fn a_baseline_without_signals_warns_only_when_signals_are_on() {
        let without = [signal_entry(None), signal_entry(None)];
        assert!(missing_signals_warning(&without, true).is_some());
        assert!(missing_signals_warning(&without, false).is_none());
        // Nothing to compare is not a baseline without signals.
        assert!(missing_signals_warning(&[], true).is_none());
        // One recorded entry means the baseline has signals.
        let mixed = [
            signal_entry(None),
            signal_entry(Some(cargo_crap::signals::Signals::default())),
        ];
        assert!(missing_signals_warning(&mixed, true).is_none());
    }

    use cargo_crap::merge::StrayFiles;

    fn diag(
        analyzed: usize,
        lcov: usize,
        matched: usize,
    ) -> ScopeDiagnostics {
        ScopeDiagnostics {
            analyzed_files: analyzed,
            lcov_files: lcov,
            matched_files: matched,
            source_only: StrayFiles {
                count: analyzed - matched,
                examples: Vec::new(),
            },
            lcov_only: StrayFiles {
                count: lcov - matched,
                examples: Vec::new(),
            },
        }
    }

    #[test]
    fn scope_warning_lead_zero_overlap_gets_strongest_wording() {
        let lead = scope_warning_lead(&diag(3, 5, 0));
        assert!(lead.contains("no overlap"), "got: {lead}");
        assert!(lead.contains("3 analyzed"), "got: {lead}");
        assert!(lead.contains("5 in LCOV"), "got: {lead}");
    }

    #[test]
    fn scope_warning_lead_below_half_escalates() {
        let lead = scope_warning_lead(&diag(40, 20, 15));
        assert!(lead.contains("only 15 of 40"), "got: {lead}");
        assert!(
            lead.contains("likely describe different scopes"),
            "got: {lead}"
        );
    }

    #[test]
    fn scope_warning_lead_exactly_half_stays_neutral() {
        // 2*matched == analyzed is NOT "fewer than half" — the escalation
        // must not fire (kills < → <= on the *2 comparison).
        let lead = scope_warning_lead(&diag(4, 4, 2));
        assert!(!lead.contains("only"), "got: {lead}");
        assert!(
            lead.contains("4 analyzed, 4 in LCOV, 2 matched"),
            "got: {lead}"
        );
    }

    #[test]
    fn scope_warning_lead_majority_overlap_stays_neutral() {
        let lead = scope_warning_lead(&diag(10, 12, 9));
        assert!(
            lead.contains("10 analyzed, 12 in LCOV, 9 matched"),
            "got: {lead}"
        );
        assert!(lead.contains("verify your --lcov path"), "got: {lead}");
    }

    #[test]
    fn stray_lines_empty_side_produces_nothing() {
        let s = StrayFiles {
            count: 0,
            examples: Vec::new(),
        };
        assert_eq!(stray_lines("label", &s), Vec::<String>::new());
    }

    #[test]
    fn stray_lines_no_tail_when_examples_cover_the_count() {
        let s = StrayFiles {
            count: 2,
            examples: vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
        };
        let lines = stray_lines("file(s) missing", &s);
        assert_eq!(
            lines,
            vec![
                "  2 file(s) missing:".to_string(),
                "    src/a.rs".to_string(),
                "    src/b.rs".to_string(),
            ]
        );
    }

    #[test]
    fn stray_lines_tail_counts_the_truncated_remainder() {
        let s = StrayFiles {
            count: 13,
            examples: (0..10).map(|i| PathBuf::from(format!("f{i}.rs"))).collect(),
        };
        let lines = stray_lines("stray", &s);
        assert_eq!(lines.len(), 12, "header + 10 examples + tail");
        assert_eq!(lines.last().unwrap(), "    ... and 3 more");
    }

    fn member(
        name: &str,
        dir: &str,
    ) -> WorkspaceMember {
        WorkspaceMember {
            name: name.into(),
            dir: PathBuf::from(dir),
        }
    }

    #[test]
    fn select_members_filters_and_preserves_discovery_order() {
        let discovered = vec![
            member("alpha", "/ws/crates/alpha"),
            member("beta", "/ws/crates/beta"),
            member("gamma", "/ws/crates/gamma"),
        ];
        let selected = select_members(&discovered, &["gamma".into(), "alpha".into()]).unwrap();
        let names: Vec<&str> = selected.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["alpha", "gamma"],
            "discovery order, not request order"
        );
    }

    #[test]
    fn select_members_deduplicates_repeated_selections() {
        let discovered = vec![member("alpha", "/ws/a"), member("beta", "/ws/b")];
        let selected = select_members(&discovered, &["alpha".into(), "alpha".into()]).unwrap();
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn select_members_rejects_unknown_names_listing_available() {
        let discovered = vec![member("alpha", "/ws/a"), member("beta", "/ws/b")];
        let err = select_members(&discovered, &["alpha".into(), "zeta".into()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown package(s): zeta"), "got: {err}");
        assert!(
            err.contains("available workspace members: alpha, beta"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_workspace_metadata_extracts_root_target_and_members() {
        let json = serde_json::json!({
            "workspace_root": "/ws",
            "target_directory": "/ws/build-out",
            "packages": [
                {"name": "alpha", "manifest_path": "/ws/crates/alpha/Cargo.toml"},
            ]
        });
        let ws = parse_workspace_metadata(json.to_string().as_bytes()).unwrap();
        assert_eq!(ws.root, PathBuf::from("/ws"));
        assert_eq!(ws.target, PathBuf::from("/ws/build-out"));
        assert_eq!(ws.members.len(), 1);
        assert_eq!(ws.members[0].name, "alpha");
        assert_eq!(ws.members[0].dir, PathBuf::from("/ws/crates/alpha"));
    }

    #[test]
    fn the_cache_is_on_unless_a_switch_turns_it_off() {
        assert!(cache_enabled(false, None));
        assert!(cache_enabled(false, Some(true)));
        assert!(!cache_enabled(false, Some(false)));
        assert!(!cache_enabled(true, None));
        assert!(!cache_enabled(true, Some(true)));
    }

    #[test]
    fn parse_workspace_metadata_rejects_missing_root_and_empty_packages() {
        let no_root = serde_json::json!({"packages": []});
        let err = parse_workspace_metadata(no_root.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(err.contains("workspace_root"), "got: {err}");

        let no_target = serde_json::json!({"workspace_root": "/ws", "packages": []});
        let err = parse_workspace_metadata(no_target.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(err.contains("target_directory"), "got: {err}");

        let empty = serde_json::json!({
            "workspace_root": "/ws",
            "target_directory": "/ws/target",
            "packages": []
        });
        let err = parse_workspace_metadata(empty.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(err.contains("no packages"), "got: {err}");
    }

    /// Scope with a root package plus two `crates/*` members, `alpha`
    /// selected — the layout of the reviewer-flagged root-leak scenario.
    fn root_layout_scope() -> MemberScope {
        let discovered = vec![
            member("rooty", "/ws"),
            member("alpha", "/ws/crates/alpha"),
            member("beta", "/ws/crates/beta"),
        ];
        let selected = vec![member("alpha", "/ws/crates/alpha")];
        MemberScope::new(Path::new("/ws"), &discovered, &selected)
    }

    #[test]
    fn scope_prefix_tier_deepest_member_owns_the_file() {
        let scope = root_layout_scope();
        // Root-package file: under /ws but under no child member → rooty
        // owns it, rooty is unselected → dropped. This is the baseline
        // leak the glob approach could not express.
        assert!(scope.is_out_of_scope(Path::new("/ws/src/lib.rs")));
        // Selected member's file: alpha out-prefixes rooty (deeper).
        assert!(!scope.is_out_of_scope(Path::new("/ws/crates/alpha/src/lib.rs")));
        // Unselected sibling member.
        assert!(scope.is_out_of_scope(Path::new("/ws/crates/beta/src/lib.rs")));
    }

    #[test]
    fn scope_window_tier_matches_cross_root_baselines() {
        let scope = root_layout_scope();
        // Same workspace recorded under a different checkout root (spec 21).
        assert!(scope.is_out_of_scope(Path::new("/ci/build/repo/crates/beta/src/lib.rs")));
        assert!(!scope.is_out_of_scope(Path::new("/ci/build/repo/crates/alpha/src/lib.rs")));
        // Cross-root root-package file: the empty rel never claims it —
        // conservative keep (it may show in `removed`, never mis-drops).
        assert!(!scope.is_out_of_scope(Path::new("/ci/build/repo/src/lib.rs")));
    }

    #[test]
    fn scope_window_tie_with_a_selected_member_keeps_the_entry() {
        // `beta` is both an unselected member dir and an ordinary module
        // dir inside the selected `frontend` — the collision that broke
        // the unanchored-glob approach. Single-component windows tie;
        // the selected match must win.
        let discovered = vec![
            member("frontend", "/ws/frontend"),
            member("beta", "/ws/beta"),
        ];
        let selected = vec![member("frontend", "/ws/frontend")];
        let scope = MemberScope::new(Path::new("/ws"), &discovered, &selected);
        assert!(!scope.is_out_of_scope(Path::new("/ci/x/frontend/src/beta/mod.rs")));
        // A genuine beta file still drops.
        assert!(scope.is_out_of_scope(Path::new("/ci/x/beta/src/lib.rs")));
    }

    #[test]
    fn scope_window_deeper_rel_wins_over_its_parent() {
        let discovered = vec![
            member("parent", "/ws/parent"),
            member("nested", "/ws/parent/nested"),
        ];
        let selected = vec![member("nested", "/ws/parent/nested")];
        let scope = MemberScope::new(Path::new("/ws"), &discovered, &selected);
        assert!(!scope.is_out_of_scope(Path::new("/ci/r/parent/nested/src/lib.rs")));
        assert!(scope.is_out_of_scope(Path::new("/ci/r/parent/src/lib.rs")));
    }

    #[test]
    fn scope_normalizes_windows_separators() {
        let scope = root_layout_scope();
        assert!(scope.is_out_of_scope(Path::new(r"C:\ci\repo\crates\beta\src\lib.rs")));
        assert!(!scope.is_out_of_scope(Path::new(r"C:\ci\repo\crates\alpha\src\lib.rs")));
    }

    #[test]
    fn components_contain_requires_contiguous_full_window() {
        let hay = ["ws", "crates", "beta", "src"];
        assert!(components_contain(&hay, &["crates".into(), "beta".into()]));
        assert!(!components_contain(
            &hay,
            &["crates".into(), "alpha".into()]
        ));
        assert!(
            !components_contain(&hay, &["ws".into(), "beta".into()]),
            "non-contiguous components must not match"
        );
        assert!(!components_contain(&hay, &[]), "empty window never matches");
    }

    #[test]
    fn nested_member_excludes_covers_only_roots_beneath_this_one() {
        let discovered = vec![
            member("parent", "/ws/parent"),
            member("nested", "/ws/parent/nested"),
            member("deep", "/ws/parent/sub/deep"),
            member("sibling", "/ws/sibling"),
        ];
        let mut globs = nested_member_excludes(Path::new("/ws/parent"), &discovered);
        globs.sort();
        assert_eq!(
            globs,
            ["nested/**", "sub/deep/**"],
            "nested roots excluded relative to the walk root; siblings untouched"
        );
        assert!(
            nested_member_excludes(Path::new("/ws/sibling"), &discovered).is_empty(),
            "a leaf member excludes nothing"
        );
        assert!(
            nested_member_excludes(Path::new("/ws/parent/nested"), &discovered).is_empty(),
            "a member never excludes itself"
        );
    }

    #[test]
    fn display_limits_take_the_flag_over_the_config_unless_all_rows() {
        let none = (None, None);
        assert_eq!(display_limits(false, none, none), none);
        assert_eq!(
            display_limits(false, (Some(1.0), None), (Some(2.0), Some(3))),
            (Some(1.0), Some(3))
        );
        assert_eq!(
            display_limits(false, (None, Some(4)), (Some(2.0), Some(3))),
            (Some(2.0), Some(4))
        );
        assert_eq!(
            display_limits(true, (Some(1.0), Some(4)), (Some(2.0), Some(3))),
            none
        );
    }

    proptest::proptest! {
        #[test]
        fn all_rows_clears_every_limit_and_otherwise_the_flag_wins(
            all_rows: bool,
            cli_min in proptest::option::of(0u32..100),
            cli_top in proptest::option::of(1usize..10),
            cfg_min in proptest::option::of(0u32..100),
            cfg_top in proptest::option::of(1usize..10),
        ) {
            let (min, top) = display_limits(
                all_rows,
                (cli_min.map(f64::from), cli_top),
                (cfg_min.map(f64::from), cfg_top),
            );
            if all_rows {
                proptest::prop_assert_eq!((min, top), (None, None));
            } else {
                proptest::prop_assert_eq!(min, cli_min.or(cfg_min).map(f64::from));
                proptest::prop_assert_eq!(top, cli_top.or(cfg_top));
            }
        }
    }

    #[test]
    fn validate_merged_values_truth_table() {
        // (epsilon, jobs) → ok? Negative epsilon and zero jobs are the two
        // values the config file could previously smuggle past the
        // CLI-only checks.
        assert!(
            validate_merged_values(0.0, None, &dup_settings(0.82, 0.5), 1.0).is_ok(),
            "zero epsilon is valid"
        );
        assert!(validate_merged_values(0.01, Some(4), &dup_settings(0.82, 0.5), 1.0).is_ok());
        assert!(
            validate_merged_values(-0.001, None, &dup_settings(0.82, 0.5), 1.0).is_err(),
            "negative epsilon"
        );
        assert!(
            validate_merged_values(0.01, Some(0), &dup_settings(0.82, 0.5), 1.0).is_err(),
            "zero jobs"
        );
        let err = validate_merged_values(-1.0, Some(0), &dup_settings(0.82, 0.5), 1.0)
            .unwrap_err()
            .to_string();
        assert!(err.contains("epsilon"), "epsilon is checked first: {err}");
    }

    #[test]
    fn a_baseline_weight_mismatch_names_both_weights_and_what_it_means() {
        let warning = try_weight_mismatch_warning(1.0, 0.0).expect("the weights differ");
        assert!(warning.starts_with("warning: "), "{warning}");
        assert!(
            warning.contains("recorded with try-weight 1") && warning.contains("uses 0"),
            "names the baseline's weight, then the run's: {warning}"
        );
        assert!(
            warning.contains("weight change, not code changes"),
            "{warning}"
        );
        let warning = try_weight_mismatch_warning(0.5, 2.0).expect("the weights differ");
        assert!(
            warning.contains("try-weight 0.5") && warning.contains("uses 2"),
            "{warning}"
        );
    }

    #[test]
    fn matching_try_weights_produce_no_warning() {
        for weight in [0.0, 0.5, 1.0, 100.0] {
            assert_eq!(try_weight_mismatch_warning(weight, weight), None);
        }
    }

    /// Duplicate settings with only the two range-checked values chosen.
    fn dup_settings(
        threshold: f64,
        triage_floor: f64,
    ) -> DupSettings {
        DupSettings {
            enabled: false,
            threshold,
            min_nodes: cargo_crap::config::DEFAULT_DUP_MIN_NODES,
            triage_enabled: false,
            triage_provider: default_provider(),
            triage_model: default_provider().default_model().to_owned(),
            triage_floor,
            triage_max_requests: None,
        }
    }

    #[test]
    fn an_unset_provider_is_the_default_one() {
        let provider = resolve_triage_provider(None).expect("the default");
        assert_eq!(provider.id(), default_provider().id());
    }

    #[test]
    fn every_registered_provider_resolves_to_itself() {
        for id in cargo_crap::duplicates::triage::provider::ids() {
            let provider = resolve_triage_provider(Some(id)).expect("registered");
            assert_eq!(provider.id(), id);
        }
    }

    #[test]
    fn an_unknown_provider_is_rejected_naming_the_choices() {
        let err = resolve_triage_provider(Some("acme"))
            .err()
            .expect("acme is no provider")
            .to_string();
        assert_eq!(
            err,
            "invalid duplicates.triage.provider (config): \"acme\" is not a provider; \
             expected one of: typesafe, openai"
        );
    }

    /// Registered ids half the time, near-misses and noise the rest.
    fn provider_ids() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::strategy::Strategy;
        proptest::prop_oneof![
            proptest::sample::select(
                cargo_crap::duplicates::triage::provider::ids().collect::<Vec<_>>()
            )
            .prop_map(|id: &str| id.to_owned()),
            "[a-zA-Z._ -]{0,12}",
        ]
    }

    proptest::proptest! {
        /// A provider id validates exactly when the registry holds it, and a
        /// rejection lists exactly the registered ids.
        #[test]
        fn a_provider_validates_exactly_when_registered(id in provider_ids()) {
            use cargo_crap::duplicates::triage::provider::{by_id, ids};
            let result = resolve_triage_provider(Some(&id));
            proptest::prop_assert_eq!(result.is_ok(), by_id(&id).is_some());
            if let Err(err) = result {
                let listed = err.to_string();
                let choices = listed.rsplit("expected one of: ").next().expect("a list");
                let expected: Vec<&str> = ids().collect();
                proptest::prop_assert_eq!(choices, expected.join(", "));
            }
        }
    }

    #[test]
    fn validate_merged_values_accepts_a_triage_floor_from_zero_to_one() {
        for floor in [0.0, 0.5, 1.0] {
            assert!(
                validate_merged_values(0.01, None, &dup_settings(0.82, floor), 1.0).is_ok(),
                "floor {floor} is valid"
            );
        }
    }

    #[test]
    fn validate_merged_values_rejects_an_out_of_range_triage_floor() {
        for floor in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            let err = validate_merged_values(0.01, None, &dup_settings(0.82, floor), 1.0)
                .expect_err("an out-of-range floor must be rejected")
                .to_string();
            assert!(
                err.contains("confidence-floor") && err.contains("0.0") && err.contains("1.0"),
                "floor {floor} names the key and the range: {err}"
            );
        }
    }

    #[test]
    fn validate_merged_values_accepts_any_try_weight_from_zero_to_one_hundred() {
        // Zero is the point of the knob, and above one is legitimate for
        // auditing error-handling-heavy code, up to the cap.
        for weight in [0.0, -0.0, f64::MIN_POSITIVE, 0.5, 1.0, 3.0, 100.0] {
            assert!(
                validate_merged_values(0.01, None, &dup_settings(0.82, 0.5), weight).is_ok(),
                "try-weight {weight} is valid"
            );
        }
    }

    #[test]
    fn validate_merged_values_rejects_negative_nan_infinite_and_over_cap_try_weight() {
        for weight in [
            -0.5,
            -f64::MIN_POSITIVE,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            // Finite, but large enough to overflow CC or CRAP to inf/NaN,
            // which serialize as `null` and slip past the gate.
            100.5,
            1e308,
            f64::MAX,
        ] {
            let err = validate_merged_values(0.01, None, &dup_settings(0.82, 0.5), weight)
                .expect_err("an out-of-domain try-weight must be rejected")
                .to_string();
            assert!(
                err.contains("try-weight") && err.contains("non-negative number"),
                "try-weight {weight} names the key and the domain: {err}"
            );
        }
    }

    #[test]
    fn resolve_color_truth_table() {
        // (no_color, force_color, output_is_file, stdout_is_tty) → expected.
        // The TTY-on rows are unreachable from CLI tests (no TTY in CI), so
        // this table is what kills mutants inside resolve_color.
        let cases = [
            ((false, false, false, true), true),   // plain terminal → on
            ((false, false, false, false), false), // piped stdout → off
            ((false, false, true, true), false),   // --output file wins over TTY
            ((false, false, true, false), false),
            ((false, true, true, false), true), // FORCE_COLOR beats file+pipe
            ((false, true, false, false), true),
            ((true, true, false, true), false), // NO_COLOR beats FORCE_COLOR
            ((true, false, false, true), false),
        ];
        for ((no_color, force, file, tty), expected) in cases {
            assert_eq!(
                resolve_color(no_color, force, file, tty),
                expected,
                "resolve_color({no_color}, {force}, {file}, {tty})"
            );
        }
    }

    #[test]
    fn require_baseline_only_errs_when_flag_set_without_baseline() {
        // Flag set + no baseline → error mentioning the flag.
        let err = require_baseline(true, false, "--show-unchanged").unwrap_err();
        assert!(
            err.to_string()
                .contains("--show-unchanged requires --baseline")
        );
        // Every other combination is fine.
        assert!(require_baseline(true, true, "--show-unchanged").is_ok());
        assert!(require_baseline(false, false, "--show-unchanged").is_ok());
        assert!(require_baseline(false, true, "--show-unchanged").is_ok());
    }

    #[test]
    fn sort_arg_maps_to_matching_sort_order() {
        // Kills: From<SortArg> collapsing to Default::default() (always Crap).
        assert_eq!(SortOrder::from(SortArg::Crap), SortOrder::Crap);
        assert_eq!(SortOrder::from(SortArg::File), SortOrder::File);
    }

    #[test]
    fn resolve_missing_cli_wins_then_config_then_pessimistic() {
        assert_eq!(
            resolve_missing(
                Some(MissingPolicy::Optimistic),
                Some(MissingCoveragePolicy::Skip)
            ),
            MissingCoveragePolicy::Optimistic
        );
        assert_eq!(
            resolve_missing(None, Some(MissingCoveragePolicy::Skip)),
            MissingCoveragePolicy::Skip
        );
        assert_eq!(
            resolve_missing(None, None),
            MissingCoveragePolicy::Pessimistic
        );
    }

    #[test]
    fn resolve_bool_cli_wins_then_config_then_false() {
        // CLI flag set → true regardless of config.
        assert!(resolve_bool(true, Some(false)));
        assert!(resolve_bool(true, None));
        // CLI flag unset → fall back to config value.
        assert!(resolve_bool(false, Some(true)));
        assert!(!resolve_bool(false, Some(false)));
        // Neither set → default false.
        assert!(!resolve_bool(false, None));
    }

    // --- effective_excludes (spec 14 precedence) ---------------------------

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn effective_excludes_keep_user_globs_apart_from_the_defaults() {
        let out = effective_excludes(
            false,
            Some(strs(&["benches/**"])),
            strs(&["src/legacy/**"]),
            strs(&["src/generated/**"]),
        );
        assert_eq!(out.defaults, strs(&["benches/**"]));
        assert_eq!(out.user, strs(&["src/legacy/**", "src/generated/**"]));
        assert_eq!(out.cli_start, 1, "the CLI globs follow the config ones");
    }

    #[test]
    fn only_command_line_globs_that_matched_nothing_are_unmatched() {
        let excludes = Excludes {
            defaults: strs(&["tests/**"]),
            user: strs(&["config/**", "hit/**", "miss/**"]),
            cli_start: 1,
        };
        assert_eq!(
            unmatched_cli_excludes(&excludes, &[false, true, false]),
            ["miss/**"]
        );
        assert_eq!(
            unmatched_cli_excludes(&excludes, &[false, false, true]),
            ["hit/**"]
        );
    }

    #[test]
    fn a_command_line_glob_given_twice_is_unmatched_once() {
        let excludes = Excludes {
            defaults: Vec::new(),
            user: strs(&["miss/**", "miss/**"]),
            cli_start: 0,
        };
        assert_eq!(
            unmatched_cli_excludes(&excludes, &[false, false]),
            ["miss/**"]
        );
    }

    #[test]
    fn the_no_match_warning_names_the_base_of_the_mode() {
        assert_eq!(
            no_match_warning("x/**", false),
            "warning: --exclude 'x/**' excluded no file; globs are relative to `--path`"
        );
        assert_eq!(
            no_match_warning("x/**", true),
            "warning: --exclude 'x/**' excluded no file; globs are relative to \
             each member's directory or the workspace root"
        );
    }

    #[test]
    fn effective_excludes_defaults_to_builtin_list() {
        let out = effective_excludes(false, None, vec![], vec![]);
        assert_eq!(flat(&out), strs(&["tests/**", "benches/**", "examples/**"]));
    }

    #[test]
    fn effective_excludes_config_replaces_builtin_list() {
        let out = effective_excludes(
            false,
            Some(strs(&["benches/**", "examples/**"])),
            vec![],
            vec![],
        );
        assert_eq!(flat(&out), strs(&["benches/**", "examples/**"]));
    }

    #[test]
    fn effective_excludes_empty_config_list_disables_defaults() {
        let out = effective_excludes(false, Some(vec![]), vec![], vec![]);
        assert_eq!(flat(&out), Vec::<String>::new());
    }

    #[test]
    fn effective_excludes_flag_overrides_config_replacement() {
        let out = effective_excludes(true, Some(strs(&["tests/**"])), vec![], vec![]);
        assert_eq!(flat(&out), Vec::<String>::new());
    }

    #[test]
    fn effective_excludes_user_globs_append_never_replace() {
        let out = effective_excludes(
            false,
            None,
            strs(&["src/legacy/**"]),
            strs(&["src/generated/**"]),
        );
        assert_eq!(
            flat(&out),
            strs(&[
                "tests/**",
                "benches/**",
                "examples/**",
                "src/legacy/**",
                "src/generated/**",
            ])
        );
    }

    // --- BaselineFilter (spec 18) -------------------------------------------

    fn baseline_entry(
        file: &str,
        function: &str,
    ) -> cargo_crap::merge::CrapEntry {
        cargo_crap::merge::CrapEntry {
            file: PathBuf::from(file),
            function: function.to_string(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            signals: None,
            fingerprint: None,
            signal_spans: cargo_crap::signals::Spans::default(),
            uncovered: Vec::new(),
        }
    }

    fn names(entries: &[cargo_crap::merge::CrapEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.function.as_str()).collect()
    }

    fn flat(excludes: &Excludes) -> Vec<String> {
        excludes
            .defaults
            .iter()
            .chain(&excludes.user)
            .cloned()
            .collect()
    }

    fn defaults_only(globs: &[&str]) -> Excludes {
        Excludes {
            defaults: strs(globs),
            user: Vec::new(),
            cli_start: 0,
        }
    }

    struct Selection {
        scope: CargoScope,
        allow: AllowSet,
    }

    impl Selection {
        fn prepare(
            &self,
            entries: Vec<cargo_crap::merge::CrapEntry>,
            duplicate_groups: Option<Vec<Vec<Location>>>,
        ) -> PreparedBaseline {
            BaselineFilter::new(&self.scope, &self.allow).prepare(Baseline {
                entries,
                duplicate_groups,
                try_weight: cargo_crap::config::DEFAULT_TRY_WEIGHT,
            })
        }
    }

    /// A filter over `dirs`, each walked under `excludes`, as
    /// the CLI does for the current run.
    fn filter_for(
        excludes: &Excludes,
        allow: &[String],
        dirs: Vec<PathBuf>,
        workspace_root: Option<&Path>,
    ) -> Result<Selection> {
        let roots: Vec<ScanRoot> = dirs
            .into_iter()
            .map(|dir| {
                ScanRoot::new(
                    dir,
                    excludes,
                    Vec::new(),
                    workspace_root.map(Path::to_path_buf),
                )
            })
            .collect::<Result<_>>()?;
        Ok(Selection {
            scope: CargoScope::new(&roots, None),
            allow: AllowSet::compile(allow)?,
        })
    }

    #[test]
    fn relative_baseline_paths_resolve_below_the_workspace_root() {
        let at = |file: &str| Location {
            file: PathBuf::from(file),
            start_line: 1,
            end_line: 9,
            name: "f".into(),
        };
        let baseline = || Baseline {
            entries: vec![
                baseline_entry("crates/a/src/lib.rs", "rel"),
                baseline_entry("/abs/x.rs", "abs"),
                baseline_entry("crates\\b\\src\\lib.rs", "windows"),
            ],
            try_weight: 1.0,
            duplicate_groups: Some(vec![vec![at("crates/a/src/lib.rs"), at("/abs/x.rs")]]),
        };
        let files = |b: &Baseline| -> (Vec<PathBuf>, Vec<PathBuf>) {
            (
                b.entries.iter().map(|e| e.file.clone()).collect(),
                b.duplicate_groups.as_ref().unwrap()[0]
                    .iter()
                    .map(|m| m.file.clone())
                    .collect(),
            )
        };
        let mut resolved = baseline();
        resolve_relative_paths(&mut resolved, Some(Path::new("/ws")));
        let expected = vec![
            PathBuf::from("/ws/crates/a/src/lib.rs"),
            PathBuf::from("/abs/x.rs"),
        ];
        let (entries, members) = files(&resolved);
        assert_eq!(entries[..2], expected[..]);
        assert_eq!(entries[2], PathBuf::from("/ws/crates/b/src/lib.rs"));
        assert_eq!(members, expected);
        let mut untouched = baseline();
        resolve_relative_paths(&mut untouched, None);
        assert_eq!(files(&untouched), files(&baseline()));
    }

    #[test]
    fn baseline_filter_matches_a_user_glob_below_a_single_root() {
        let filter = filter_for(
            &Excludes {
                defaults: Vec::new(),
                user: strs(&["src/generated/**"]),
                cli_start: 0,
            },
            &[],
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![
            baseline_entry("./src/generated/api.rs", "dropped"),
            baseline_entry("./src/lib.rs", "kept"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    fn workspace_filter(excludes: &Excludes) -> Selection {
        filter_for(
            excludes,
            &[],
            vec![
                PathBuf::from("/ws/agent"),
                PathBuf::from("/ws/tests/harness"),
            ],
            Some(Path::new("/ws")),
        )
        .unwrap()
    }

    #[test]
    fn baseline_filter_matches_a_user_glob_below_the_workspace_root() {
        let filter = workspace_filter(&Excludes {
            defaults: Vec::new(),
            user: strs(&["agent/**"]),
            cli_start: 0,
        });
        let entries = vec![
            baseline_entry("/ws/agent/src/lib.rs", "dropped"),
            baseline_entry("/ws/tests/harness/src/lib.rs", "kept"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_never_matches_a_default_below_the_workspace_root() {
        let filter = workspace_filter(&defaults_only(&["tests/**"]));
        let entries = vec![
            baseline_entry("/ws/tests/harness/src/lib.rs", "kept"),
            baseline_entry("/ws/agent/tests/it.rs", "dropped"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_drops_root_level_target_dirs() {
        let filter = filter_for(
            &defaults_only(&["tests/**", "benches/**", "examples/**"]),
            &[],
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![
            baseline_entry("./src/lib.rs", "kept"),
            baseline_entry("./tests/integration.rs", "test_helper"),
            baseline_entry("benches/bench.rs", "bench_helper"),
            baseline_entry("examples/demo.rs", "example_helper"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_keeps_nested_tests_dir_inside_src() {
        // Excludes are root-relative: `tests/**` must not match a module
        // directory that happens to be named tests/ deeper in the tree.
        let filter = filter_for(
            &defaults_only(&["tests/**"]),
            &[],
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![baseline_entry("./src/tests/helpers.rs", "nested_helper")];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["nested_helper"]);
    }

    #[test]
    fn baseline_filter_drops_duplicate_members_only_by_the_exclude_globs() {
        // `--allow` hides CRAP rows, and the duplicate scan never reads it:
        // a member it names is still in the current run's groups.
        let filter = filter_for(
            &defaults_only(&["tests/**"]),
            &strs(&["skip_me", "gen/**"]),
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let at = |file: &str, name: &str| Location {
            file: PathBuf::from(file),
            start_line: 1,
            end_line: 9,
            name: name.to_string(),
        };
        let kept = vec![
            at("src/a.rs", "g"),
            at("src/b.rs", "skip_me"),
            at("gen/c.rs", "h"),
        ];
        let groups = vec![
            vec![at("tests/it.rs", "helper"), at("src/a.rs", "g")],
            kept.clone(),
        ];
        let prepared = filter.prepare(Vec::new(), Some(groups));
        let groups = prepared.as_baseline().duplicate_groups.as_ref().unwrap();
        assert_eq!(groups, &[vec![at("src/a.rs", "g")], kept]);
    }

    #[test]
    fn baseline_scores_keep_only_selected_member_files_that_the_walk_included() {
        let excludes = defaults_only(&["tests/**"]);
        let root = PathBuf::from("/ws/crates/alpha");
        let mut selection = filter_for(&excludes, &[], vec![root], Some(Path::new("/ws"))).unwrap();
        selection.scope.members = Some(root_layout_scope());
        let rows = vec![
            baseline_entry("/ws/crates/alpha/src/lib.rs", "selected"),
            baseline_entry("/ws/crates/alpha/tests/helper.rs", "excluded"),
            baseline_entry("/ws/crates/beta/src/lib.rs", "unselected"),
        ];
        let prepared = selection.prepare(rows, None);
        assert_eq!(names(&prepared.as_baseline().entries), ["selected"]);
    }

    #[test]
    fn a_duplicate_record_is_narrowed_to_the_selected_members() {
        let at = |file: &str, name: &str| Location {
            file: PathBuf::from(file),
            start_line: 1,
            end_line: 9,
            name: name.to_string(),
        };
        let mut filter =
            filter_for(&Excludes::default(), &[], vec![PathBuf::from("/ws")], None).unwrap();
        filter.scope.members = Some(root_layout_scope());
        let groups = vec![
            vec![
                at("/ws/crates/alpha/src/lib.rs", "f"),
                at("/ws/crates/beta/src/lib.rs", "g"),
            ],
            vec![
                at("/ws/crates/alpha/src/a.rs", "h"),
                at("/ws/crates/alpha/src/b.rs", "k"),
            ],
        ];
        let prepared = filter.prepare(Vec::new(), Some(groups));
        let groups = prepared.as_baseline().duplicate_groups.as_ref().unwrap();
        assert_eq!(
            groups,
            &[
                vec![at("/ws/crates/alpha/src/lib.rs", "f")],
                vec![
                    at("/ws/crates/alpha/src/a.rs", "h"),
                    at("/ws/crates/alpha/src/b.rs", "k"),
                ],
            ]
        );
    }

    #[test]
    fn nobody_is_named_when_triage_gave_no_verdict() {
        let section = |triage| DuplicateSection {
            pairs: None,
            groups: None,
            novelty: None,
            triage,
            untriaged: 0,
            provider: "OpenAI API",
            model: "gpt-x",
        };
        assert!(section(Some(vec![None, None])).triaged_by().is_none());
        assert!(section(None).triaged_by().is_none());
        let answered = section(Some(vec![
            None,
            Some(Assessment::Uncertain { confidence: 0.4 }),
        ]));
        assert!(answered.triaged_by().is_some());
    }

    #[test]
    fn only_a_baseline_without_a_duplicate_record_is_warned_about() {
        let baseline = |duplicate_groups| Baseline {
            entries: vec![],
            try_weight: cargo_crap::config::DEFAULT_TRY_WEIGHT,
            duplicate_groups,
        };
        let at = |name: &str| duplicates::extract::Location {
            file: PathBuf::from("src/a.rs"),
            start_line: 1,
            end_line: 9,
            name: name.to_string(),
        };
        let found = [duplicates::group::DuplicateGroup::of_pair(&DuplicatePair {
            first: at("f"),
            second: at("g"),
            score: 0.9,
        })];
        let warning =
            missing_record_warning(Some(&found), Some(&baseline(None))).expect("a warning");
        assert!(warning.contains("no duplicate record"), "{warning}");
        assert!(warning.contains("--duplicates"), "{warning}");
        assert!(!warning.contains(';'), "{warning}");
        let record = baseline(Some(vec![]));
        assert_eq!(missing_record_warning(Some(&found), Some(&record)), None);
        assert_eq!(missing_record_warning(Some(&found), None), None);
        assert_eq!(
            missing_record_warning(Some(&[]), Some(&baseline(None))),
            None
        );
        assert_eq!(missing_record_warning(None, Some(&baseline(None))), None);
    }

    #[test]
    fn triage_asks_about_every_group_without_a_record() {
        assert_eq!(asked(None, 3), [0, 1, 2]);
    }

    #[test]
    fn triage_asks_only_about_new_and_grown_groups() {
        use duplicates::novelty::{Comparison, Novelty};
        let comparison = Comparison {
            novelty: vec![
                Novelty::Unchanged,
                Novelty::New,
                Novelty::Grown { from: 2 },
                Novelty::Unchanged,
            ],
            resolved: 0,
        };
        assert_eq!(asked(Some(&comparison), 4), [1, 2]);
    }

    #[test]
    fn verdicts_go_back_to_the_groups_that_were_asked_about() {
        let verdict = |confidence| Assessment::Uncertain { confidence };
        assert_eq!(
            spread(&[1, 2], vec![Some(verdict(0.1)), Some(verdict(0.2))], 4),
            [None, Some(verdict(0.1)), Some(verdict(0.2)), None]
        );
        // A group the budget left unasked keeps its place, without a verdict.
        assert_eq!(
            spread(&[0, 3], vec![None, Some(verdict(0.3))], 4),
            [None, None, None, Some(verdict(0.3))]
        );
    }

    #[test]
    fn baseline_filter_normalizes_backslash_paths() {
        // Baseline written on Windows: separators must be normalized before
        // glob matching, as in merge's path_key.
        let filter = filter_for(
            &defaults_only(&["tests/**"]),
            &[],
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![baseline_entry("tests\\integration.rs", "test_helper")];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert!(entries.is_empty(), "backslash path must match tests/**");
    }

    #[test]
    fn baseline_filter_strips_longest_member_root_first() {
        // Workspace mode: globs apply relative to each member root, so
        // crates/foo/tests/it.rs is tested as tests/it.rs. A nested member
        // (crates/foo/sub) must claim its files before the shallower root.
        let filter = filter_for(
            &defaults_only(&["tests/**"]),
            &[],
            vec![PathBuf::from("crates/foo"), PathBuf::from("crates/foo/sub")],
            None,
        )
        .unwrap();
        let entries = vec![
            baseline_entry("crates/foo/tests/it.rs", "member_test"),
            baseline_entry("crates/foo/sub/tests/it.rs", "nested_member_test"),
            baseline_entry("crates/foo/src/lib.rs", "kept"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_applies_name_allow_patterns() {
        let filter = filter_for(
            &Excludes::default(),
            &strs(&["generated_*"]),
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![
            baseline_entry("src/codegen.rs", "generated_parse_v1"),
            baseline_entry("src/lib.rs", "kept"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_applies_path_allow_patterns() {
        // Path-shaped allow patterns use the same suffix matching as
        // filter::allow, so they work against absolute baseline paths too.
        let filter = filter_for(
            &Excludes::default(),
            &strs(&["src/generated/**"]),
            vec![PathBuf::from(".")],
            None,
        )
        .unwrap();
        let entries = vec![
            baseline_entry("/abs/proj/src/generated/api.rs", "dropped"),
            baseline_entry("/abs/proj/src/lib.rs", "kept"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_with_no_patterns_is_a_no_op() {
        let filter = filter_for(&Excludes::default(), &[], vec![PathBuf::from(".")], None).unwrap();
        let entries = vec![
            baseline_entry("tests/integration.rs", "test_helper"),
            baseline_entry("src/lib.rs", "lib_fn"),
        ];
        let prepared = filter.prepare(entries, None);
        let entries = &prepared.as_baseline().entries;
        assert_eq!(names(entries), ["test_helper", "lib_fn"]);
    }
}
