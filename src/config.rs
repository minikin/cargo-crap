//! Optional persistent configuration via `.cargo-crap.toml`.
//!
//! The file is searched for by walking up from the current working directory.
//! CLI flags always take precedence over values in the config file — the
//! config only fills in values the user did not explicitly provide.
//!
//! ## Example `.cargo-crap.toml`
//!
//! ```toml
//! threshold = 30.0
//! fail-above = true
//! missing = "pessimistic"
//! # Appends to the default exclusions (tests/**, benches/**, examples/**).
//! exclude = ["src/generated/**"]
//! # Replaces the default-exclude list. `[]` disables it entirely.
//! default-excludes = ["benches/**", "examples/**"]
//! # `allow` accepts both function-name globs and path globs (any entry
//! # containing `/` or `**` is treated as a path glob).
//! allow = ["generated::*", "src/generated/**"]
//! # Final entry ordering: "crap" (default) or "file" (stable for baselines).
//! sort = "file"
//! # Show Unchanged rows in --baseline mode (human / markdown).
//! show_unchanged = true
//! ```

use crate::merge::{MissingCoveragePolicy, SortOrder};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Persistent settings loaded from `.cargo-crap.toml`.
///
/// All fields are optional — only the keys present in the config file override
/// the built-in defaults. CLI flags take precedence over every field here.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    /// CRAP score above which a function is considered "crappy".
    pub threshold: Option<f64>,

    /// Exit non-zero if any function's CRAP score exceeds `threshold`.
    pub fail_above: Option<bool>,

    /// How to handle functions with no coverage data.
    /// One of `"pessimistic"` (default), `"optimistic"`, or `"skip"`.
    pub missing: Option<MissingCoveragePolicy>,

    /// Glob patterns for source files to skip (relative to `--path`).
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Replaces the built-in default-exclude list (`tests/**`, `benches/**`,
    /// `examples/**`) wholesale. `[]` disables default exclusions; a subset
    /// re-includes some directories; a superset extends the defaults.
    /// Accepted as `default-excludes` (house style) or `default_excludes`.
    /// Unlike `exclude`, which appends, this key replaces.
    #[serde(alias = "default_excludes")]
    pub default_excludes: Option<Vec<String>>,

    /// Only show the top N crappiest functions.
    pub top: Option<usize>,

    /// Only show functions with a CRAP score at or above this value.
    pub min: Option<f64>,

    /// Glob patterns for function names to suppress from the report.
    /// Supports `*` (matches any chars including `::`) and `?`.
    /// Example: `"Foo::*"` suppresses all methods on `Foo`.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Exit non-zero if any function regressed since `--baseline`.
    pub fail_regression: Option<bool>,

    /// Maximum number of threads used by `analyze_tree` for parallel file
    /// analysis. `None` lets rayon size the pool to the host. Must be
    /// non-zero when set.
    pub jobs: Option<usize>,

    /// Tolerance for the regression detector. Score deltas with absolute
    /// value at or below this are reported as `Unchanged`. Must be
    /// non-negative when set.
    pub epsilon: Option<f64>,

    /// Final ordering of report entries. One of `"crap"` (default, CRAP score
    /// descending) or `"file"` (`(file, function, line)` ascending).
    pub sort: Option<SortOrder>,

    /// In `--baseline` mode, show `Unchanged` rows in the human and markdown
    /// tables. Defaults to false: only changed functions are listed.
    /// Accepted as `show-unchanged` (house style) or `show_unchanged`.
    #[serde(alias = "show_unchanged")]
    pub show_unchanged: Option<bool>,
}

/// Walk up from `start` until `.cargo-crap.toml` is found.
///
/// Returns [`Config::default`] when no config file exists anywhere in the
/// directory hierarchy — this means the tool works without any config file.
pub fn load(start: &Path) -> Result<Config> {
    let mut dir = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    loop {
        let candidate = dir.join(".cargo-crap.toml");
        if candidate.exists() {
            let raw = fs::read_to_string(&candidate)
                .with_context(|| format!("reading {}", candidate.display()))?;
            let cfg: Config =
                toml::from_str(&raw).with_context(|| format!("parsing {}", candidate.display()))?;
            return Ok(cfg);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return Ok(Config::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_config(
        dir: &Path,
        content: &str,
    ) {
        let mut f = fs::File::create(dir.join(".cargo-crap.toml")).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn missing_config_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load(dir.path()).unwrap();
        assert!(cfg.threshold.is_none());
        assert!(cfg.fail_above.is_none());
        assert!(cfg.missing.is_none());
        assert!(cfg.exclude.is_empty());
        assert!(cfg.allow.is_empty());
    }

    #[test]
    fn config_file_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            r#"
threshold = 20.0
fail-above = true
missing = "optimistic"
exclude = ["tests/**"]
allow = ["Foo::*"]
"#,
        );
        let cfg = load(dir.path()).unwrap();
        assert_eq!(cfg.threshold, Some(20.0));
        assert_eq!(cfg.fail_above, Some(true));
        assert_eq!(cfg.missing, Some(MissingCoveragePolicy::Optimistic));
        assert_eq!(cfg.exclude, ["tests/**"]);
        assert_eq!(cfg.allow, ["Foo::*"]);
    }

    #[test]
    fn default_excludes_absent_means_none() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "threshold = 20.0\n");
        let cfg = load(dir.path()).unwrap();
        assert!(cfg.default_excludes.is_none());
    }

    #[test]
    fn default_excludes_kebab_case_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "default-excludes = [\"benches/**\", \"examples/**\"]\n",
        );
        let cfg = load(dir.path()).unwrap();
        assert_eq!(
            cfg.default_excludes.as_deref(),
            Some(&["benches/**".to_string(), "examples/**".to_string()][..])
        );
    }

    #[test]
    fn default_excludes_snake_case_alias_is_parsed() {
        // Spec 14 scenarios write the key as `default_excludes`; both
        // spellings must work despite `deny_unknown_fields`.
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "default_excludes = [\"tests/**\"]\n");
        let cfg = load(dir.path()).unwrap();
        assert_eq!(
            cfg.default_excludes.as_deref(),
            Some(&["tests/**".to_string()][..])
        );
    }

    #[test]
    fn default_excludes_empty_list_is_some_empty() {
        // `[]` must be distinguishable from "key absent": it disables the
        // built-in defaults rather than falling back to them.
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "default-excludes = []\n");
        let cfg = load(dir.path()).unwrap();
        assert_eq!(cfg.default_excludes.as_deref(), Some(&[][..]));
    }

    #[test]
    fn config_is_found_by_walking_up() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "threshold = 15.0\n");
        let subdir = dir.path().join("src");
        fs::create_dir(&subdir).unwrap();
        // Start from a subdirectory — should walk up and find the config.
        let cfg = load(&subdir).unwrap();
        assert_eq!(cfg.threshold, Some(15.0));
    }

    #[test]
    fn sort_and_show_unchanged_are_parsed() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "sort = \"file\"\nshow_unchanged = true\n");
        let cfg = load(dir.path()).unwrap();
        assert_eq!(cfg.sort, Some(SortOrder::File));
        assert_eq!(cfg.show_unchanged, Some(true));
    }

    #[test]
    fn sort_and_show_unchanged_absent_means_none() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "threshold = 20.0\n");
        let cfg = load(dir.path()).unwrap();
        assert!(cfg.sort.is_none());
        assert!(cfg.show_unchanged.is_none());
    }

    #[test]
    fn unknown_key_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "unknown-key = true\n");
        let err = load(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("parsing"),
            "expected parse error, got: {err}"
        );
    }
}
