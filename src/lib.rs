//! Compute the **CRAP** (Change Risk Anti-Patterns) metric for Rust projects.
//!
//! The score combines cyclomatic complexity and test coverage into one
//! number. It is high when code is both hard to understand *and* poorly
//! tested.
//!
//! ```text
//! CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
//! ```
//!
//! Consequences of the formula:
//!
//! - A trivial function (CC = 1, 100% covered) scores **1.0**, the lower
//!   bound.
//! - At 100% coverage the quadratic term drops out and **CRAP equals CC**, so
//!   matching columns mean a fully covered function. The tests cap the risk;
//!   the complexity is still there.
//! - Above CC ≈ 30 no amount of coverage keeps the score under the default
//!   threshold of 30, since at full coverage the score is CC itself. Being
//!   tested does not make a very large function clean.
//!
//! # Quick start
//!
//! Scoring a single function:
//!
//! ```rust
//! use cargo_crap::score::crap;
//!
//! // Trivial, fully covered → always 1.0 (the lower bound).
//! assert_eq!(crap(1.0, 100.0), 1.0);
//!
//! // Moderately complex, half covered: 16 × 0.5³ + 4 = 6.0
//! assert_eq!(crap(4.0, 50.0), 6.0);
//!
//! // Savoia & Evans worked example: CC=6, 0% → 6² × 1³ + 6 = 42.0
//! assert_eq!(crap(6.0, 0.0), 42.0);
//!
//! // CC=12, untested → 12² + 12 = 156, well past the threshold of 30.
//! assert_eq!(crap(12.0, 0.0), 156.0);
//! ```
//!
//! # Embedding the full pipeline
//!
//! The library exposes the pipeline the `cargo crap` CLI runs, for putting
//! CRAP gating into a custom CI tool, an editor plugin or a refactoring
//! advisor.
//!
//! ```no_run
//! use cargo_crap::{
//!     complexity, coverage,
//!     merge::{MissingCoveragePolicy, merge},
//!     report::{RenderOptions, render},
//! };
//! use std::io;
//!
//! // 1. Walk the source tree and compute cyclomatic complexity per function.
//! //    The second argument is a list of glob patterns to exclude.
//! let fns = complexity::analyze_tree(
//!     std::path::Path::new("src"),
//!     &[] as &[&str],
//! )?;
//!
//! // 2. Parse the LCOV report produced by `cargo llvm-cov --lcov`.
//! let cov = coverage::parse_lcov(std::path::Path::new("lcov.info"))?;
//!
//! // 3. Join complexity with coverage. Functions with no coverage data are
//! //    treated as 0% covered (the pessimistic default, safest for CI gates).
//! let entries = merge(fns, cov, MissingCoveragePolicy::Pessimistic).entries;
//!
//! // 4. Render the human-readable table to stdout. `RenderOptions`
//! //    defaults to the CLI defaults: threshold 30, human format.
//! render(&entries, &RenderOptions::default(), &mut io::stdout())?;
//!
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Threshold gate
//!
//! The usual CI gate exits non-zero when any function exceeds a threshold.
//! Check the entries yourself:
//!
//! ```no_run
//! use cargo_crap::{
//!     complexity, coverage,
//!     merge::{MissingCoveragePolicy, merge},
//! };
//!
//! let fns = complexity::analyze_tree(
//!     std::path::Path::new("src"),
//!     &[] as &[&str],
//! )?;
//! let cov = coverage::parse_lcov(std::path::Path::new("lcov.info"))?;
//! let entries = merge(fns, cov, MissingCoveragePolicy::Pessimistic).entries;
//!
//! let threshold = 30.0_f64;
//! let crappy: Vec<_> = entries.iter().filter(|e| e.crap > threshold).collect();
//! if !crappy.is_empty() {
//!     eprintln!("{} function(s) exceed CRAP threshold {threshold}:", crappy.len());
//!     for e in &crappy {
//!         eprintln!("  {} — CRAP {:.1} ({}:{})", e.function, e.crap,
//!                   e.file.display(), e.line);
//!     }
//!     std::process::exit(1);
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Baseline comparison (delta mode)
//!
//! Teams usually gate on regressions against a saved baseline instead. Load a
//! previous run's JSON output and pass it to [`delta::compute_delta`]:
//!
//! ```no_run
//! use cargo_crap::{
//!     complexity, coverage,
//!     delta::{DEFAULT_EPSILON, compute_delta, load_baseline},
//!     merge::{MissingCoveragePolicy, merge},
//!     report::{RenderOptions, render_delta},
//! };
//! use std::io;
//!
//! let fns = complexity::analyze_tree(
//!     std::path::Path::new("src"),
//!     &[] as &[&str],
//! )?;
//! let cov = coverage::parse_lcov(std::path::Path::new("lcov.info"))?;
//! let entries = merge(fns, cov, MissingCoveragePolicy::Pessimistic).entries;
//!
//! // Load baseline saved by a previous `--format json --output baseline.json` run.
//! let baseline = load_baseline(std::path::Path::new("baseline.json"))?;
//! let report = compute_delta(&entries, &baseline, DEFAULT_EPSILON);
//!
//! // Exit non-zero if any function regressed.
//! if report.regression_count() > 0 {
//!     render_delta(&report, &RenderOptions::default(), &mut io::stdout())?;
//!     std::process::exit(1);
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Modules
//!
//! | Module | Role |
//! |---|---|
//! | [`score`] | The CRAP formula and the `Clean`/`Crappy` classifier. No I/O. |
//! | [`complexity`] | `syn`-based AST walker. Produces `(file, function, span, CC)` per function. |
//! | [`coverage`] | LCOV parser. Produces `(file, line) → hit-count` maps. |
//! | [`merge`] | Joins complexity with coverage. Handles all path-matching cases. |
//! | [`delta`] | Baseline comparison. Computes per-function deltas and regression counts. |
//! | [`duplicates`] | Structural duplicate detection: normalize each function's AST, fingerprint its subtrees, compare pairs by Jaccard similarity. Optional triage of the pairs by a `TypeSafe` model lives in `duplicates::triage` (the HTTP client needs the `triage` feature). |
//! | [`report`] | Renders `Vec<CrapEntry>` or `DeltaReport` as a human table, JSON, GitHub annotations, Markdown, a PR comment, SARIF, or a Shields.io badge. |
//! | [`config`] | Loads `.cargo-crap.toml` by walking up from CWD. |

pub mod complexity;
pub mod config;
pub mod coverage;
pub mod delta;
pub mod duplicates;
pub mod merge;
pub mod report;
pub mod score;
