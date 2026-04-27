//! `cargo crap` — CLI entrypoint.
//!
//! Invoked two ways:
//! - Directly as `cargo-crap [args...]`.
//! - As a cargo subcommand `cargo crap [args...]`, in which case cargo
//!   invokes us as `cargo-crap crap [args...]`. We strip that leading
//!   `crap` argument below.

use anyhow::{Context, Result, bail};
use cargo_crap::{
    complexity, coverage,
    delta::{compute_delta, load_baseline},
    merge::{MissingCoveragePolicy, merge},
    report::{Format, crappy_count, render, render_delta, render_delta_summary, render_summary},
    score::DEFAULT_THRESHOLD,
};
use clap::{Parser, ValueEnum};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use indicatif::{ProgressBar, ProgressStyle};
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
    /// GitHub-Flavored Markdown table — suitable for PR comment bots.
    Markdown,
}

impl From<FormatArg> for Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Human => Self::Human,
            FormatArg::Json => Self::Json,
            FormatArg::Github => Self::GitHub,
            FormatArg::Markdown => Self::Markdown,
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

/// Walk source trees for either a single path or all workspace members.
fn collect_complexity(
    workspace: bool,
    path: &std::path::Path,
    excludes: &[String],
) -> Result<Vec<complexity::FunctionComplexity>> {
    if workspace {
        let roots = workspace_roots()?;
        let mut all = Vec::new();
        for root in &roots {
            let fns = complexity::analyze_tree(root, excludes)
                .with_context(|| format!("analyzing {}", root.display()))?;
            all.extend(fns);
        }
        Ok(all)
    } else {
        complexity::analyze_tree(path, excludes)
            .with_context(|| format!("analyzing {}", path.display()))
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
fn load_coverage(
    lcov: Option<&PathBuf>
) -> Result<std::collections::HashMap<std::path::PathBuf, cargo_crap::coverage::FileCoverage>> {
    match lcov {
        Some(path) => coverage::parse_lcov(path)
            .with_context(|| format!("parsing LCOV file {}", path.display())),
        None => Ok(Default::default()),
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

/// Discover all workspace member roots via `cargo metadata`.
///
/// Returns a list of `manifest_path` parent directories (one per member crate)
/// that should be walked for Rust source files. Falls back to `[cwd]` if
/// `cargo metadata` fails — e.g., when running outside a Cargo workspace.
fn workspace_roots() -> Result<Vec<PathBuf>> {
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

    let roots: Vec<PathBuf> = meta["packages"]
        .as_array()
        .context("`cargo metadata` output missing `packages`")?
        .iter()
        .filter_map(|pkg| {
            pkg["manifest_path"]
                .as_str()
                .and_then(|p| PathBuf::from(p).parent().map(|d| d.to_path_buf()))
        })
        .collect();

    if roots.is_empty() {
        bail!("`cargo metadata` returned no packages");
    }
    Ok(roots)
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
    Ok(())
}

/// Render the final report and return `(has_crappy, has_regression)` for exit-code decisions.
fn do_render(
    entries: &[cargo_crap::merge::CrapEntry],
    baseline: Option<&PathBuf>,
    threshold: f64,
    format: Format,
    summary: bool,
    out: &mut dyn Write,
) -> Result<(bool, bool)> {
    if let Some(baseline_path) = baseline {
        let baseline_data = load_baseline(baseline_path)?;
        let report = compute_delta(entries, &baseline_data);
        let has_crappy = crappy_count(entries, threshold) > 0;
        let has_regression = report.regression_count() > 0;
        if summary {
            render_delta_summary(&report, out)?;
        } else {
            render_delta(&report, threshold, format, out)?;
        }
        Ok((has_crappy, has_regression))
    } else {
        let has_crappy = crappy_count(entries, threshold) > 0;
        if summary {
            render_summary(entries, threshold, out)?;
        } else {
            render(entries, threshold, format, out)?;
        }
        Ok((has_crappy, false))
    }
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

    let mut effective_exclude = config.exclude;
    effective_exclude.extend(cli.exclude);
    let mut effective_allow = config.allow;
    effective_allow.extend(cli.allow);

    // --- Analysis ---
    let pb = spinner("Analyzing source files…");
    let fns = collect_complexity(cli.workspace, &cli.path, &effective_exclude)?;

    pb.set_message("Parsing coverage report…");
    let coverage = load_coverage(cli.lcov.as_ref())?;
    pb.finish_and_clear();

    // --- Merge + filters ---
    let merge_result = merge(fns, coverage, missing_policy);
    warn_unmapped(&merge_result.unmapped_files);
    let mut entries = merge_result.entries;
    apply_filters(
        &mut entries,
        &effective_allow,
        cli.min.or(config.min),
        cli.top.or(config.top),
    )?;

    // --- Render ---
    let mut out_box = open_output(cli.output.as_ref())?;
    let (has_crappy, has_regression) = do_render(
        &entries,
        cli.baseline.as_ref(),
        threshold,
        cli.format.into(),
        cli.summary,
        out_box.as_mut(),
    )?;

    if (fail_above && has_crappy) || (fail_regression && has_regression) {
        std::process::exit(1);
    }
    Ok(())
}
