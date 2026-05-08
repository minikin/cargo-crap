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
    merge::{MissingCoveragePolicy, merge},
    report::{
        Format, SourceLinks, crappy_count, render, render_delta, render_delta_summary,
        render_summary,
    },
    score::DEFAULT_THRESHOLD,
};
use clap::{Parser, ValueEnum};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
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
    reason = "the bools come from clap-derived `--flag` switches (--workspace, --summary, --fail-above, --fail-regression); not a struct-design smell"
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

    /// Glob patterns for files to skip (relative to `--path`).
    /// Use `**` to cross directory boundaries. May be repeated.
    ///
    /// Examples: `--exclude 'tests/**'`  `--exclude 'src/generated/**'`
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

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

    /// Suppress functions whose names match these glob patterns.
    /// Supports `*` (matches any sequence of characters, including `::`)
    /// and `?`. May be repeated.
    ///
    /// Examples: `--allow 'Foo::*'`  `--allow 'generated_*'`
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
}

impl From<FormatArg> for Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Human => Self::Human,
            FormatArg::Json => Self::Json,
            FormatArg::Github => Self::GitHub,
            FormatArg::Markdown => Self::Markdown,
            FormatArg::PrComment => Self::PrComment,
        }
    }
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

/// Build a `GlobSet` for matching function names from allow-list patterns.
///
/// Unlike file-path exclusions, we do NOT set `literal_separator` — this lets
/// `*` match across `::` so that `"Foo::*"` suppresses all methods on `Foo`.
fn build_allow_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .build()
            .with_context(|| format!("invalid allow pattern: {pat:?}"))?;
        builder.add(glob);
    }
    builder.build().context("building allow glob set")
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
/// `analyze_tree` walk respects user-set bounds. In `--workspace` mode also
/// discovers all members via `cargo metadata` and walks each member's root.
fn analyze_sources(
    workspace: bool,
    path: &std::path::Path,
    excludes: &[String],
    jobs: Option<usize>,
) -> Result<(Vec<complexity::FunctionComplexity>, Vec<WorkspaceMember>)> {
    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .with_context(|| format!("configuring rayon thread pool to {n} threads"))?;
    }
    if workspace {
        let members = workspace_members()?;
        let mut all = Vec::new();
        for m in &members {
            let fns = complexity::analyze_tree(&m.dir, excludes)
                .with_context(|| format!("analyzing {}", m.dir.display()))?;
            all.extend(fns);
        }
        Ok((all, members))
    } else {
        let fns = complexity::analyze_tree(path, excludes)
            .with_context(|| format!("analyzing {}", path.display()))?;
        Ok((fns, Vec::new()))
    }
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
fn apply_filters(
    entries: &mut Vec<cargo_crap::merge::CrapEntry>,
    allow_patterns: &[String],
    min: Option<f64>,
    top: Option<usize>,
) -> Result<()> {
    if !allow_patterns.is_empty() {
        let allow_set = build_allow_set(allow_patterns)?;
        entries.retain(|e| !allow_set.is_match(&e.function));
    }
    if let Some(min) = min {
        entries.retain(|e| e.crap >= min);
    }
    if let Some(top) = top {
        entries.truncate(top);
    }
    Ok(())
}

/// Parse the LCOV file if one was provided, returning an empty map otherwise.
fn load_coverage(lcov: Option<&PathBuf>) -> Result<HashMap<PathBuf, FileCoverage>> {
    match lcov {
        Some(path) => coverage::parse_lcov(path)
            .with_context(|| format!("parsing LCOV file {}", path.display())),
        None => Ok(HashMap::new()),
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
fn workspace_members() -> Result<Vec<WorkspaceMember>> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .context("running `cargo metadata`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`cargo metadata` failed: {stderr}");
    }

    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parsing `cargo metadata` output")?;

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
    Ok(members)
}

/// Emit a stderr warning listing source files with no LCOV match.
///
/// `unmapped_files` is empty when no `--lcov` was given (merge guarantees this),
/// so the call is always safe and the guard lives here rather than at the call site.
fn warn_unmapped(files: &[std::path::PathBuf]) {
    if files.is_empty() {
        return;
    }
    let n = files.len();
    eprintln!(
        "warning: {} source file{} had no matching entry in the LCOV report \
         — verify your --lcov path or coverage tool configuration:",
        n,
        if n == 1 { "" } else { "s" },
    );
    for f in files {
        eprintln!("  {}", f.display());
    }
}

/// Validate argument combinations that clap cannot express as declarative rules.
fn validate_args(cli: &Cli) -> Result<()> {
    if !cli.workspace && !cli.path.exists() {
        bail!("path does not exist: {}", cli.path.display());
    }
    if cli.fail_regression && cli.baseline.is_none() {
        bail!("--fail-regression requires --baseline");
    }
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
}

/// Render the final report and return `(has_crappy, has_regression)` for exit-code decisions.
fn do_render(
    entries: &[cargo_crap::merge::CrapEntry],
    baseline: Option<&PathBuf>,
    opts: &RenderOpts,
    out: &mut dyn Write,
) -> Result<(bool, bool)> {
    if let Some(baseline_path) = baseline {
        let baseline_data = load_baseline(baseline_path)?;
        let report = compute_delta(entries, &baseline_data, opts.epsilon);
        let has_crappy = crappy_count(entries, opts.threshold) > 0;
        let has_regression = report.regression_count() > 0;
        if opts.summary {
            render_delta_summary(&report, out)?;
        } else {
            render_delta(&report, opts.threshold, opts.format, opts.links, out)?;
        }
        Ok((has_crappy, has_regression))
    } else {
        let has_crappy = crappy_count(entries, opts.threshold) > 0;
        if opts.summary {
            render_summary(entries, opts.threshold, out)?;
        } else {
            render(entries, opts.threshold, opts.format, opts.links, out)?;
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

fn main() -> Result<()> {
    let cli = Cli::parse_from(strip_cargo_subcommand(std::env::args().collect()));
    validate_args(&cli)?;

    // --- Load config (optional; defaults if .cargo-crap.toml not found) ---
    let cwd = std::env::current_dir().unwrap_or_else(|_| cli.path.clone());
    let config = cargo_crap::config::load(&cwd)?;

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

    let fail_above = cli.fail_above || config.fail_above.unwrap_or(false);
    let fail_regression = cli.fail_regression || config.fail_regression.unwrap_or(false);

    let epsilon = cli
        .epsilon
        .or(config.epsilon)
        .unwrap_or(cargo_crap::delta::DEFAULT_EPSILON);

    let mut effective_exclude = config.exclude;
    effective_exclude.extend(cli.exclude);
    let mut effective_allow = config.allow;
    effective_allow.extend(cli.allow);

    // --- Analysis ---
    let pb = spinner("Analyzing source files…");
    let (fns, members) = analyze_sources(
        cli.workspace,
        &cli.path,
        &effective_exclude,
        cli.jobs.or(config.jobs),
    )?;

    pb.set_message("Parsing coverage report…");
    let coverage = load_coverage(cli.lcov.as_ref())?;
    pb.finish_and_clear();

    // --- Merge + filters ---
    let merge_result = merge(fns, coverage, missing_policy);
    warn_unmapped(&merge_result.unmapped_files);
    let mut entries = merge_result.entries;
    assign_crate_names(&mut entries, &members);
    apply_filters(
        &mut entries,
        &effective_allow,
        cli.min.or(config.min),
        cli.top.or(config.top),
    )?;

    // --- Render ---
    let mut out_box = open_output(cli.output.as_ref())?;
    let links = resolve_source_links(cli.repo_url, cli.commit_ref);
    let opts = RenderOpts {
        threshold,
        epsilon,
        format: cli.format.into(),
        summary: cli.summary,
        links: links.as_ref(),
    };
    let (has_crappy, has_regression) =
        do_render(&entries, cli.baseline.as_ref(), &opts, out_box.as_mut())?;

    if (fail_above && has_crappy) || (fail_regression && has_regression) {
        std::process::exit(1);
    }
    Ok(())
}
