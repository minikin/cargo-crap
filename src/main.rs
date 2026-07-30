//! `cargo crap` — CLI entrypoint.
//!
//! Invoked two ways:
//! - Directly as `cargo-crap [args...]`.
//! - As a cargo subcommand `cargo crap [args...]`, in which case cargo
//!   invokes us as `cargo-crap crap [args...]`. We strip that leading
//!   `crap` argument below.

use anyhow::{Context, Result, bail};
use cargo_crap::{
    complexity,
    coverage::{self, FileCoverage},
    delta::{compute_delta, load_baseline},
    merge::{MissingCoveragePolicy, ScopeDiagnostics, SortOrder, merge, sort_entries},
    report::{
        Format, SourceLinks, crappy_count, render, render_delta, render_delta_summary,
        render_summary, set_color_enabled,
    },
    score::DEFAULT_THRESHOLD,
};
use clap::{Parser, ValueEnum};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
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
    reason = "the bools come from clap-derived `--flag` switches (--workspace, --summary, --fail-above, --fail-regression, --no-default-excludes, --show-unchanged); not a struct-design smell"
)]
struct Cli {
    /// Path to an LCOV coverage file (e.g. from `cargo llvm-cov --lcov --output-path lcov.info`).
    ///
    /// If omitted, every function is scored as if it had 0% coverage — useful
    /// for a first look at complexity distribution but not a real CRAP run.
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
    /// exactly the selected members (spec 25). Unknown names fail before
    /// analysis; duplicates are deduplicated; `--path` is ignored.
    /// Conflicts with `--workspace`, which already means "all members".
    #[arg(
        short = 'p',
        long = "package",
        value_name = "NAME",
        conflicts_with = "workspace"
    )]
    package: Vec<String>,

    /// Glob patterns for files to skip (relative to `--path`).
    /// Use `**` to cross directory boundaries. May be repeated.
    /// Appends to the default exclusions — see `--no-default-excludes`.
    ///
    /// Examples: `--exclude 'src/generated/**'`  `--exclude '**/build.rs'`
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Disable the built-in default exclusions (`tests/**`, `benches/**`,
    /// `examples/**`), analyzing those directories like any other source.
    /// Overrides the `default-excludes` config key.
    #[arg(long)]
    no_default_excludes: bool,

    /// CRAP score above which a function is considered "crappy".
    /// Falls back to `.cargo-crap.toml` → built-in default (30).
    #[arg(long)]
    threshold: Option<f64>,

    /// Only print functions with a CRAP score above this cutoff.
    #[arg(long, value_name = "SCORE")]
    min: Option<f64>,

    /// Limit the report to the top N crappiest functions.
    #[arg(long, value_name = "N")]
    top: Option<usize>,

    /// How to handle functions with complexity data but no coverage data.
    /// Falls back to `.cargo-crap.toml` → built-in default (pessimistic).
    #[arg(long, value_enum)]
    missing: Option<MissingPolicy>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,

    /// Print only aggregate statistics (total analyzed, crappy count, worst
    /// offender) — no per-function table. Compatible with all `--format` values
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
    /// from the report — distinct from `--exclude`, which skips files at
    /// walk time.
    ///
    /// Examples: `--allow 'Foo::*'`  `--allow 'generated_*'`
    ///           `--allow 'src/generated/**'`  `--allow 'tests/**'`
    #[arg(long, value_name = "GLOB")]
    allow: Vec<String>,

    /// JSON baseline from a previous `--format json` run. When provided the
    /// report shows per-function deltas (regressions, improvements, new).
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

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
    /// descending; `file` sorts by `(file, function, line)` ascending — stable
    /// across score changes, ideal for committed JSON baselines. `--top` always
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
    /// GitHub Actions workflow commands — one `::warning` per crappy function.
    Github,
    /// GitHub-Flavored Markdown table — exhaustive, no row caps. Use this for
    /// archived artifacts and docs pages.
    Markdown,
    /// Opinionated PR-comment markdown — hides unchanged rows, caps each
    /// section, collapses non-critical info into `<details>` blocks. Designed
    /// for the GitHub PR comment use case where readability beats completeness.
    PrComment,
    /// SARIF 2.1.0 JSON for upload to GitHub Code Scanning, VS Code, and
    /// other static-analysis tools. Each crappy function becomes one
    /// warning-level result. Incompatible with `--baseline`.
    Sarif,
    /// Shields.io endpoint-badge JSON — counts functions above `--threshold`.
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

/// Assemble the effective exclude list (spec 14 precedence): the default
/// list (built-in, or replaced by `default-excludes`, or emptied by
/// `--no-default-excludes`), followed by config and CLI `exclude` globs,
/// which always append and never replace.
fn effective_excludes(
    no_default_excludes: bool,
    config_default_excludes: Option<Vec<String>>,
    config_exclude: Vec<String>,
    cli_exclude: Vec<String>,
) -> Vec<String> {
    let mut out = if no_default_excludes {
        Vec::new()
    } else {
        config_default_excludes
            .unwrap_or_else(|| DEFAULT_EXCLUDES.iter().map(ToString::to_string).collect())
    };
    out.extend(config_exclude);
    out.extend(cli_exclude);
    out
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

/// True when an `--allow` entry should be treated as a path glob rather than a
/// function-name glob. The rule is intentionally simple: contains `/` or `**`.
/// This is documented in the `--allow` help text so users can predict the
/// classification.
fn is_path_allow_pattern(pattern: &str) -> bool {
    pattern.contains('/') || pattern.contains("**")
}

/// Build a `GlobSet` for matching function names from allow-list patterns.
///
/// Unlike file-path exclusions, we do NOT set `literal_separator` — this lets
/// `*` match across `::` so that `"Foo::*"` suppresses all methods on `Foo`.
fn build_allow_set(patterns: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .build()
            .with_context(|| format!("invalid allow pattern: {pat:?}"))?;
        builder.add(glob);
    }
    builder.build().context("building allow glob set")
}

/// Build a `GlobSet` for matching file paths. `what` names the option in
/// error messages (`"allow"` / `"exclude"`).
///
/// Mirrors `analyze_tree`'s exclude build (`literal_separator(true)` so `*`
/// stays within one path component and `**` is required to cross directories).
fn build_path_set(
    patterns: &[&str],
    what: &str,
) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid {what} pattern: {pat:?}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .with_context(|| format!("building {what} path glob set"))
}

/// True if any path-glob in `set` matches a component-suffix of `path`.
///
/// Walking suffixes lets a relative pattern like `src/generated/**` match an
/// absolute file path like `/home/u/project/src/generated/foo.rs` without the
/// caller having to know which form `entry.file` takes.
fn path_set_matches_suffix(
    set: &GlobSet,
    path: &Path,
) -> bool {
    if set.is_empty() {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    for i in 0..components.len() {
        let suffix: PathBuf = components[i..].iter().collect();
        if set.is_match(&suffix) {
            return true;
        }
    }
    false
}

/// One Cargo workspace member: package name + the directory containing its
/// `Cargo.toml`.
#[derive(Debug, Clone)]
struct WorkspaceMember {
    name: String,
    dir: PathBuf,
}

/// Walk source trees and discover Cargo workspace members in one pass.
///
/// Caps rayon's global thread pool to `jobs` first, so the parallel
/// `analyze_tree` walk respects user-set bounds. In `--workspace` mode all
/// members discovered via `cargo metadata` are walked; with `-p/--package`
/// only the selected members are (spec 25). Either way, a member walk never
/// descends into another member's nested root — each file is analyzed once,
/// by the member that owns it.
/// What [`analyze_sources`] hands back to `main`.
struct AnalyzedSources {
    fns: Vec<complexity::FunctionComplexity>,
    /// The analyzed workspace members: all of them under `--workspace`, the
    /// selected subset under `-p/--package`, empty for single-root runs.
    members: Vec<WorkspaceMember>,
    /// Baseline scope attribution for `-p` runs (spec 25). `None` unless
    /// `-p` narrowed the analysis to a subset of the discovered members.
    member_scope: Option<MemberScope>,
}

fn analyze_sources(
    workspace: bool,
    packages: &[String],
    path: &std::path::Path,
    excludes: &[String],
    jobs: Option<usize>,
) -> Result<AnalyzedSources> {
    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .with_context(|| format!("configuring rayon thread pool to {n} threads"))?;
    }
    if !workspace && packages.is_empty() {
        let fns = complexity::analyze_tree(path, excludes)
            .with_context(|| format!("analyzing {}", path.display()))?;
        return Ok(AnalyzedSources {
            fns,
            members: Vec::new(),
            member_scope: None,
        });
    }
    analyze_workspace_members(packages, excludes)
}

/// The workspace side of [`analyze_sources`]: discover members, narrow to
/// the `-p` selection (all members when empty, i.e. `--workspace`), and
/// walk each selected member's root.
fn analyze_workspace_members(
    packages: &[String],
    excludes: &[String],
) -> Result<AnalyzedSources> {
    let (workspace_root, discovered) = workspace_members()?;
    let members = if packages.is_empty() {
        discovered.clone()
    } else {
        select_members(&discovered, packages)?
    };
    let mut fns = Vec::new();
    for m in &members {
        // Nested exclusion is computed against every discovered member, not
        // just the selected ones: an unselected nested member's files must
        // not leak into its parent's walk either.
        let mut walk_excludes = excludes.to_vec();
        walk_excludes.extend(nested_member_excludes(&m.dir, &discovered));
        let member_fns = complexity::analyze_tree(&m.dir, &walk_excludes)
            .with_context(|| format!("analyzing {}", m.dir.display()))?;
        fns.extend(member_fns);
    }
    let member_scope =
        (!packages.is_empty()).then(|| MemberScope::new(&workspace_root, &discovered, &members));
    Ok(AnalyzedSources {
        fns,
        members,
        member_scope,
    })
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

/// Apply allow-list, min-score, and top-N filters to the entry list in place.
///
/// `--allow` entries split into two buckets via [`is_path_allow_pattern`]:
/// path globs match the entry's source file, function-name globs match the
/// entry's function name. An entry is dropped when either set matches.
fn apply_filters(
    entries: &mut Vec<cargo_crap::merge::CrapEntry>,
    allow_patterns: &[String],
    min: Option<f64>,
    top: Option<usize>,
) -> Result<()> {
    if !allow_patterns.is_empty() {
        let (path_pats, name_pats): (Vec<&str>, Vec<&str>) = allow_patterns
            .iter()
            .map(String::as_str)
            .partition(|p| is_path_allow_pattern(p));
        let name_set = build_allow_set(&name_pats)?;
        let path_set = build_path_set(&path_pats, "allow")?;
        entries.retain(|e| {
            !name_set.is_match(&e.function) && !path_set_matches_suffix(&path_set, &e.file)
        });
    }
    if let Some(min) = min {
        entries.retain(|e| e.crap >= min);
    }
    if let Some(top) = top {
        entries.truncate(top);
    }
    Ok(())
}

/// Drops baseline entries that the current run's identity-based filters
/// would have dropped (spec 18): exclude globs (default + user, matched
/// relative to the analyzed roots, exactly as the walk applies them) and
/// `--allow` patterns (same path/name split as [`apply_filters`]).
///
/// Without this, changing the exclusion set between the baseline run and the
/// current run floods `removed` with functions that were not deleted — and
/// leaves those stale entries eligible for the delta's pass-2 name matcher,
/// which can pair them with unrelated new functions as phantom moves.
struct BaselineFilter {
    exclude_set: GlobSet,
    name_allow: GlobSet,
    path_allow: GlobSet,
    /// Analyzed roots (workspace member dirs, or the single `--path` root),
    /// sorted longest-first so the deepest root claims the prefix — the same
    /// longest-match rule as [`assign_crate_names`].
    roots: Vec<PathBuf>,
}

impl BaselineFilter {
    fn new(
        excludes: &[String],
        allow_patterns: &[String],
        mut roots: Vec<PathBuf>,
    ) -> Result<Self> {
        let exclude_pats: Vec<&str> = excludes.iter().map(String::as_str).collect();
        let (path_pats, name_pats): (Vec<&str>, Vec<&str>) = allow_patterns
            .iter()
            .map(String::as_str)
            .partition(|p| is_path_allow_pattern(p));
        roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
        Ok(Self {
            exclude_set: build_path_set(&exclude_pats, "exclude")?,
            name_allow: build_allow_set(&name_pats)?,
            path_allow: build_path_set(&path_pats, "allow")?,
            roots,
        })
    }

    /// True when the exclude globs match `file` once it is normalized to
    /// forward slashes (cross-platform baselines) and stripped of the
    /// deepest analyzed-root prefix (excludes are root-relative).
    fn is_excluded(
        &self,
        file: &Path,
    ) -> bool {
        if self.exclude_set.is_empty() {
            return false;
        }
        let normalized = PathBuf::from(file.to_string_lossy().replace('\\', "/"));
        let rel = self
            .roots
            .iter()
            .find_map(|root| normalized.strip_prefix(root).ok())
            .unwrap_or(&normalized);
        self.exclude_set.is_match(rel)
    }

    /// Drop every entry the current run would not have produced. Filtered
    /// entries vanish for delta purposes: they appear in no bucket
    /// (`removed` included) and cannot pair in the name-fallback pass.
    fn retain(
        &self,
        entries: &mut Vec<cargo_crap::merge::CrapEntry>,
    ) {
        entries.retain(|e| {
            !self.is_excluded(&e.file)
                && !self.name_allow.is_match(&e.function)
                && !path_set_matches_suffix(&self.path_allow, &e.file)
        });
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

/// Open the output destination: a file when `--output` is given, stdout otherwise.
fn open_output(path: Option<&PathBuf>) -> Result<Box<dyn Write>> {
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

/// Discover all workspace members via `cargo metadata`.
///
/// Returns one [`WorkspaceMember`] per member crate (name + the directory
/// containing its `Cargo.toml`). Used both to walk source trees and to
/// assign a `crate` field to each `CrapEntry` for per-crate rollup.
fn workspace_members() -> Result<(PathBuf, Vec<WorkspaceMember>)> {
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
/// anchors the members' workspace-relative dirs for [`MemberScope`]) and
/// the member list. Split from [`workspace_members`] so the parsing is
/// testable without spawning cargo.
fn parse_workspace_metadata(stdout: &[u8]) -> Result<(PathBuf, Vec<WorkspaceMember>)> {
    let meta: serde_json::Value =
        serde_json::from_slice(stdout).context("parsing `cargo metadata` output")?;

    let workspace_root = meta["workspace_root"]
        .as_str()
        .map(PathBuf::from)
        .context("`cargo metadata` output missing `workspace_root`")?;

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
    Ok((workspace_root, members))
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
    Ok(())
}

/// Bundles the render-time options threaded into [`do_render`]. Bundling
/// keeps the helper's signature under clippy's argument-count limit and makes
/// call sites readable as "what to render" + "where to render it".
struct RenderOpts<'a> {
    threshold: f64,
    epsilon: f64,
    format: Format,
    summary: bool,
    links: Option<&'a SourceLinks>,
    /// Show `Unchanged` rows in delta mode (spec 16). Only the human and
    /// markdown renderers consult it.
    show_unchanged: bool,
    /// Final entry ordering (spec 17). Applied to the `DeltaReport` so
    /// `removed` ordering is deterministic too.
    sort: SortOrder,
    /// Source/LCOV scope diagnostics (spec 24); embedded in JSON envelopes.
    diagnostics: Option<&'a ScopeDiagnostics>,
}

/// Load the `--baseline` file (if any) and filter it through the current
/// run's identity-based filters (spec 18) before delta computation. The
/// analyzed roots are the workspace member dirs, or the single `--path`
/// root outside `--workspace` mode.
fn load_filtered_baseline(
    baseline: Option<&PathBuf>,
    excludes: &[String],
    allow_patterns: &[String],
    path: &Path,
    members: &[WorkspaceMember],
    member_scope: Option<&MemberScope>,
) -> Result<Option<Vec<cargo_crap::merge::CrapEntry>>> {
    let Some(baseline_path) = baseline else {
        return Ok(None);
    };
    let mut data = load_baseline(baseline_path)?;
    let roots = if members.is_empty() {
        vec![path.to_path_buf()]
    } else {
        members.iter().map(|m| m.dir.clone()).collect()
    };
    BaselineFilter::new(excludes, allow_patterns, roots)?.retain(&mut data);
    apply_member_scope(&mut data, member_scope);
    Ok(Some(data))
}

/// Drop baseline entries owned by unselected members (spec 25): a `-p` run
/// never walked them, so they must vanish rather than flood `removed`.
/// No-op without a scope (single-root and `--workspace` runs).
fn apply_member_scope(
    data: &mut Vec<cargo_crap::merge::CrapEntry>,
    member_scope: Option<&MemberScope>,
) {
    if let Some(scope) = member_scope {
        data.retain(|e| !scope.is_out_of_scope(&e.file));
    }
}

/// Render the final report and return `(has_crappy, has_regression)` for exit-code decisions.
///
/// `baseline` arrives pre-loaded and pre-filtered (see [`BaselineFilter`]) so
/// this stays a pure render dispatcher.
fn do_render(
    entries: &[cargo_crap::merge::CrapEntry],
    baseline: Option<&[cargo_crap::merge::CrapEntry]>,
    opts: &RenderOpts,
    out: &mut dyn Write,
) -> Result<(bool, bool)> {
    if let Some(baseline_data) = baseline {
        let mut report = compute_delta(entries, baseline_data, opts.epsilon);
        report.sort(opts.sort);
        let has_crappy = crappy_count(entries, opts.threshold) > 0;
        let has_regression = report.regression_count() > 0;
        if opts.summary {
            render_delta_summary(&report, out)?;
        } else {
            render_delta(
                &report,
                opts.threshold,
                opts.format,
                opts.links,
                opts.show_unchanged,
                opts.diagnostics,
                out,
            )?;
        }
        Ok((has_crappy, has_regression))
    } else {
        let has_crappy = crappy_count(entries, opts.threshold) > 0;
        if opts.summary {
            render_summary(entries, opts.threshold, out)?;
        } else {
            render(
                entries,
                opts.threshold,
                opts.format,
                opts.links,
                opts.diagnostics,
                out,
            )?;
        }
        Ok((has_crappy, false))
    }
}

/// Resolve `--repo-url` / `--commit-ref` against the GitHub Actions env vars,
/// returning `Some(SourceLinks)` only when both pieces are present. Either
/// missing — or both missing — results in `None` (plain rendering, no links).
fn resolve_source_links(
    cli_repo_url: Option<String>,
    cli_commit_ref: Option<String>,
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
    Some(SourceLinks::new(repo_url, commit_ref))
}

/// Parse argv, validate flag combinations, and load the optional
/// `.cargo-crap.toml` (defaults when absent — the tool works without one).
fn parse_and_load_config() -> Result<(Cli, cargo_crap::config::Config)> {
    let cli = Cli::parse_from(strip_cargo_subcommand(std::env::args().collect()));
    validate_args(&cli)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| cli.path.clone());
    let config = cargo_crap::config::load(&cwd)?;
    Ok((cli, config))
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

fn run() -> Result<ExitCode> {
    let (cli, config) = parse_and_load_config()?;

    // Merge: CLI values take precedence; config fills in what's missing.
    let threshold = cli
        .threshold
        .or(config.threshold)
        .unwrap_or(DEFAULT_THRESHOLD);

    let missing_policy: MissingCoveragePolicy = cli
        .missing
        .map(Into::into)
        .or(config.missing)
        .unwrap_or(MissingCoveragePolicy::Pessimistic);

    let fail_above = resolve_bool(cli.fail_above, config.fail_above);
    let fail_regression = resolve_bool(cli.fail_regression, config.fail_regression);
    let show_unchanged = resolve_bool(cli.show_unchanged, config.show_unchanged);
    let sort_order = cli.sort.map(Into::into).or(config.sort).unwrap_or_default();

    let epsilon = cli
        .epsilon
        .or(config.epsilon)
        .unwrap_or(cargo_crap::delta::DEFAULT_EPSILON);

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
        members,
        member_scope,
    } = analyze_sources(
        cli.workspace,
        &cli.package,
        &cli.path,
        &effective_exclude,
        cli.jobs.or(config.jobs),
    )?;

    pb.set_message("Parsing coverage report…");
    let coverage = load_coverage(cli.lcov.as_ref())?;
    pb.finish_and_clear();

    // --- Merge + filters ---
    let merge_result = merge(fns, coverage, missing_policy);
    let diagnostics = merge_result.diagnostics;
    warn_scope_mismatch(diagnostics.as_ref());
    let mut entries = merge_result.entries;
    assign_crate_names(&mut entries, &members);
    apply_filters(
        &mut entries,
        &effective_allow,
        cli.min.or(config.min),
        cli.top.or(config.top),
    )?;
    // Apply the user-requested ordering after --top has selected by CRAP (spec 17).
    sort_entries(&mut entries, sort_order);

    // --- Baseline (loaded here, filtered per specs 18 + 25) ---
    let baseline_data = load_filtered_baseline(
        cli.baseline.as_ref(),
        &effective_exclude,
        &effective_allow,
        &cli.path,
        &members,
        member_scope.as_ref(),
    )?;

    // --- Render ---
    let env_set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    set_color_enabled(resolve_color(
        env_set("NO_COLOR"),
        env_set("FORCE_COLOR"),
        cli.output.is_some(),
        io::stdout().is_terminal(),
    ));
    let mut out_box = open_output(cli.output.as_ref())?;
    let links = resolve_source_links(cli.repo_url, cli.commit_ref);
    let opts = RenderOpts {
        threshold,
        epsilon,
        format: cli.format.into(),
        summary: cli.summary,
        links: links.as_ref(),
        show_unchanged,
        sort: sort_order,
        diagnostics: diagnostics.as_ref(),
    };
    let (has_crappy, has_regression) =
        do_render(&entries, baseline_data.as_deref(), &opts, out_box.as_mut())?;
    // The flush must precede the gate decision: a write failure (e.g.
    // ENOSPC) is a tool error (exit 2), never a gate verdict over a
    // truncated report (#47, spec 23).
    out_box.flush().context("flushing output")?;

    if (fail_above && has_crappy) || (fail_regression && has_regression) {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(stray_lines("label", &s).is_empty());
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
    fn parse_workspace_metadata_extracts_root_and_members() {
        let json = serde_json::json!({
            "workspace_root": "/ws",
            "packages": [
                {"name": "alpha", "manifest_path": "/ws/crates/alpha/Cargo.toml"},
            ]
        });
        let (root, members) = parse_workspace_metadata(json.to_string().as_bytes()).unwrap();
        assert_eq!(root, PathBuf::from("/ws"));
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "alpha");
        assert_eq!(members[0].dir, PathBuf::from("/ws/crates/alpha"));
    }

    #[test]
    fn parse_workspace_metadata_rejects_missing_root_and_empty_packages() {
        let no_root = serde_json::json!({"packages": []});
        let err = parse_workspace_metadata(no_root.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(err.contains("workspace_root"), "got: {err}");

        let empty = serde_json::json!({"workspace_root": "/ws", "packages": []});
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

    #[test]
    fn name_glob_classifier_keeps_function_patterns() {
        assert!(!is_path_allow_pattern("trivial"));
        assert!(!is_path_allow_pattern("Foo::*"));
        assert!(!is_path_allow_pattern("generated_*"));
        assert!(!is_path_allow_pattern("*"));
    }

    #[test]
    fn path_glob_classifier_recognizes_path_patterns() {
        assert!(is_path_allow_pattern("src/generated/**"));
        assert!(is_path_allow_pattern("tests/**"));
        assert!(is_path_allow_pattern("**/build.rs"));
        assert!(is_path_allow_pattern("a/b"));
    }

    #[test]
    fn path_set_matches_relative_pattern_against_absolute_file() {
        let set = build_path_set(&["src/generated/**"], "allow").unwrap();
        let abs = Path::new("/home/u/project/src/generated/foo.rs");
        assert!(path_set_matches_suffix(&set, abs));
    }

    #[test]
    fn path_set_does_not_match_unrelated_file() {
        let set = build_path_set(&["src/generated/**"], "allow").unwrap();
        let other = Path::new("/home/u/project/src/main.rs");
        assert!(!path_set_matches_suffix(&set, other));
    }

    #[test]
    fn empty_path_set_is_no_op() {
        let set = build_path_set(&[], "allow").unwrap();
        assert!(!path_set_matches_suffix(&set, Path::new("any/path.rs")));
    }

    #[test]
    fn path_set_respects_literal_separator() {
        // `src/*` matches direct children of `src/`, but `*` must not cross a
        // separator — so `src/generated/foo.rs` (a grandchild) does not match.
        // Crossing directories requires `**`.
        let set = build_path_set(&["src/*"], "allow").unwrap();
        assert!(!path_set_matches_suffix(
            &set,
            Path::new("/abs/proj/src/generated/foo.rs"),
        ));
    }

    // --- effective_excludes (spec 14 precedence) ---------------------------

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn effective_excludes_defaults_to_builtin_list() {
        let out = effective_excludes(false, None, vec![], vec![]);
        assert_eq!(out, strs(&["tests/**", "benches/**", "examples/**"]));
    }

    #[test]
    fn effective_excludes_config_replaces_builtin_list() {
        let out = effective_excludes(
            false,
            Some(strs(&["benches/**", "examples/**"])),
            vec![],
            vec![],
        );
        assert_eq!(out, strs(&["benches/**", "examples/**"]));
    }

    #[test]
    fn effective_excludes_empty_config_list_disables_defaults() {
        let out = effective_excludes(false, Some(vec![]), vec![], vec![]);
        assert!(out.is_empty());
    }

    #[test]
    fn effective_excludes_flag_overrides_config_replacement() {
        let out = effective_excludes(true, Some(strs(&["tests/**"])), vec![], vec![]);
        assert!(out.is_empty());
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
            out,
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
        }
    }

    fn names(entries: &[cargo_crap::merge::CrapEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.function.as_str()).collect()
    }

    #[test]
    fn baseline_filter_drops_root_level_target_dirs() {
        let filter = BaselineFilter::new(
            &strs(&["tests/**", "benches/**", "examples/**"]),
            &[],
            vec![PathBuf::from(".")],
        )
        .unwrap();
        let mut entries = vec![
            baseline_entry("./src/lib.rs", "kept"),
            baseline_entry("./tests/integration.rs", "test_helper"),
            baseline_entry("benches/bench.rs", "bench_helper"),
            baseline_entry("examples/demo.rs", "example_helper"),
        ];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_keeps_nested_tests_dir_inside_src() {
        // Excludes are root-relative: `tests/**` must not match a module
        // directory that happens to be named tests/ deeper in the tree.
        let filter =
            BaselineFilter::new(&strs(&["tests/**"]), &[], vec![PathBuf::from(".")]).unwrap();
        let mut entries = vec![baseline_entry("./src/tests/helpers.rs", "nested_helper")];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["nested_helper"]);
    }

    #[test]
    fn baseline_filter_normalizes_backslash_paths() {
        // Baseline written on Windows: separators must be normalized before
        // glob matching, as in delta's path_key.
        let filter =
            BaselineFilter::new(&strs(&["tests/**"]), &[], vec![PathBuf::from(".")]).unwrap();
        let mut entries = vec![baseline_entry("tests\\integration.rs", "test_helper")];
        filter.retain(&mut entries);
        assert!(entries.is_empty(), "backslash path must match tests/**");
    }

    #[test]
    fn baseline_filter_strips_longest_member_root_first() {
        // Workspace mode: globs apply relative to each member root, so
        // crates/foo/tests/it.rs is tested as tests/it.rs. A nested member
        // (crates/foo/sub) must claim its files before the shallower root.
        let filter = BaselineFilter::new(
            &strs(&["tests/**"]),
            &[],
            vec![PathBuf::from("crates/foo"), PathBuf::from("crates/foo/sub")],
        )
        .unwrap();
        let mut entries = vec![
            baseline_entry("crates/foo/tests/it.rs", "member_test"),
            baseline_entry("crates/foo/sub/tests/it.rs", "nested_member_test"),
            baseline_entry("crates/foo/src/lib.rs", "kept"),
        ];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_applies_name_allow_patterns() {
        let filter =
            BaselineFilter::new(&[], &strs(&["generated_*"]), vec![PathBuf::from(".")]).unwrap();
        let mut entries = vec![
            baseline_entry("src/codegen.rs", "generated_parse_v1"),
            baseline_entry("src/lib.rs", "kept"),
        ];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_applies_path_allow_patterns() {
        // Path-shaped allow patterns use the same suffix matching as
        // apply_filters, so they work against absolute baseline paths too.
        let filter =
            BaselineFilter::new(&[], &strs(&["src/generated/**"]), vec![PathBuf::from(".")])
                .unwrap();
        let mut entries = vec![
            baseline_entry("/abs/proj/src/generated/api.rs", "dropped"),
            baseline_entry("/abs/proj/src/lib.rs", "kept"),
        ];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["kept"]);
    }

    #[test]
    fn baseline_filter_with_no_patterns_is_a_no_op() {
        let filter = BaselineFilter::new(&[], &[], vec![PathBuf::from(".")]).unwrap();
        let mut entries = vec![
            baseline_entry("tests/integration.rs", "test_helper"),
            baseline_entry("src/lib.rs", "lib_fn"),
        ];
        filter.retain(&mut entries);
        assert_eq!(names(&entries), ["test_helper", "lib_fn"]);
    }
}
