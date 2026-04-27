//! Compute the **CRAP** (Change Risk Anti-Patterns) metric for Rust projects.
//!
//! The score combines cyclomatic complexity and test coverage into a single
//! number that is high when code is both hard to understand *and* poorly
//! tested — the conditions where bugs love to hide.
//!
//! ```text
//! CRAP(m) = comp(m)² × (1 − cov(m)/100)³ + comp(m)
//! ```
//!
//! A few properties worth knowing before you use the numbers:
//!
//! - A trivial function (CC = 1, 100% covered) always scores **1.0** — the
//!   lower bound.
//! - At 100% coverage the quadratic term collapses: **CRAP equals CC**. When
//!   both columns match, the function is fully covered. Tests are capping the
//!   damage, but the complexity itself remains.
//! - Above CC ≈ 30, no amount of coverage keeps the score under the default
//!   threshold of 30. The formula refuses to certify a monster method as
//!   clean just because it happens to be tested.
//!
//! # Quick start
//!
//! The simplest use case is computing the CRAP score for a single function:
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
//! // CC=12, untested → 12² + 12 = 156 — well past the threshold of 30.
//! assert_eq!(crap(12.0, 0.0), 156.0);
//! ```
//!
//! # Full pipeline — embedding in a custom tool
//!
//! The library exposes the same pipeline that the `cargo crap` CLI uses.
//! You can drive it programmatically to embed CRAP gating into a custom CI
//! tool, an editor plugin, or an automated refactoring advisor.
//!
//! ```no_run
//! use cargo_crap::{
//!     complexity, coverage,
//!     merge::{MissingCoveragePolicy, merge},
//!     report::{Format, render},
//!     score::DEFAULT_THRESHOLD,
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
//! // 4. Render the human-readable table to stdout.
//! render(&entries, DEFAULT_THRESHOLD, Format::Human, &mut io::stdout())?;
//!
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Threshold gate
//!
//! To exit non-zero when any function exceeds a threshold — the standard CI
//! gate pattern — check the entries yourself:
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
//! To detect regressions relative to a saved baseline — the recommended
//! approach for teams — use [`delta::compute_delta`] after loading a previous
//! run's JSON output:
//!
//! ```no_run
//! use cargo_crap::{
//!     complexity, coverage,
//!     delta::{compute_delta, load_baseline},
//!     merge::{MissingCoveragePolicy, merge},
//!     report::{Format, render_delta},
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
//! let report = compute_delta(&entries, &baseline);
//!
//! // Exit non-zero if any function regressed.
//! if report.regression_count() > 0 {
//!     render_delta(&report, 30.0, Format::Human, &mut io::stdout())?;
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
//! | [`report`] | Renders `Vec<CrapEntry>` or `DeltaReport` as human table, JSON, GitHub annotations, or Markdown. |
//! | [`config`] | Loads `.cargo-crap.toml` by walking up from CWD. |

pub mod complexity;
pub mod config;
pub mod coverage;
pub mod delta;
pub mod merge;
pub mod report;
pub mod score;
