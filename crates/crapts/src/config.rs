use crate::{InputError, discover::Worktree};
use cargo_crap_core::config::find_upwards;
use cargo_crap_core::entry::{MissingCoveragePolicy, SortOrder};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(crate) const MAX_TOP: u64 = 9_007_199_254_740_991;

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub threshold: Option<f64>,
    pub fail_above: Option<bool>,
    pub missing: Option<MissingCoveragePolicy>,
    pub exclude: Option<Vec<String>>,
    #[serde(alias = "default_excludes")]
    pub default_excludes: Option<Vec<String>>,
    pub allow: Option<Vec<String>>,
    pub min: Option<f64>,
    pub top: Option<u64>,
    pub sort: Option<SortOrder>,
    pub epsilon: Option<f64>,
    pub fail_regression: Option<bool>,
    #[serde(alias = "show_unchanged")]
    pub show_unchanged: Option<bool>,
    pub fail_changed: Option<bool>,
    #[serde(alias = "uncovered_hints")]
    pub uncovered_hints: Option<bool>,
}

impl Config {
    pub fn parse(
        path: &Path,
        source: &str,
    ) -> Result<Self, InputError> {
        let settings: Self = toml::from_str(source).map_err(|error| field_error(path, &error))?;
        settings.validate(path)?;
        Ok(settings)
    }

    fn validate(
        &self,
        path: &Path,
    ) -> Result<(), InputError> {
        for (name, value) in [
            ("threshold", self.threshold),
            ("min", self.min),
            ("epsilon", self.epsilon),
        ] {
            if value.is_some_and(|n| !n.is_finite()) {
                return Err(InputError::at(path, format!("{name} must be finite")));
            }
        }
        if let Some(epsilon) = self.epsilon
            && epsilon < 0.0
        {
            return Err(InputError::at(
                path,
                format!("epsilon must be at least 0, got {epsilon}"),
            ));
        }
        if let Some(top) = self.top
            && !(1..=MAX_TOP).contains(&top)
        {
            return Err(InputError::at(
                path,
                format!("top must be an integer in 1..={MAX_TOP}, got {top}"),
            ));
        }
        Ok(())
    }
}

fn field_error(
    path: &Path,
    error: &toml::de::Error,
) -> InputError {
    for (prefix, label) in [
        ("unknown field `", "unknown key"),
        ("duplicate field `", "duplicate"),
    ] {
        if let Some((name, _)) = error
            .message()
            .strip_prefix(prefix)
            .and_then(|s| s.split_once('`'))
        {
            return InputError::at(path, format!("{label} {name}\n{error}"));
        }
    }
    InputError::at(path, error)
}

pub fn load(cwd: &Path) -> Result<(Option<PathBuf>, Config), InputError> {
    let cwd = cwd.canonicalize().map_err(|err| InputError::at(cwd, err))?;
    let worktree = Worktree::probe(&cwd)?;
    load_at(&cwd, worktree.root.as_deref())
}

pub(crate) fn load_at(
    cwd: &Path,
    stop: Option<&Path>,
) -> Result<(Option<PathBuf>, Config), InputError> {
    let path = find_upwards(cwd, ".crapts.toml", stop).map_err(|err| InputError::at(cwd, err))?;
    let Some(path) = path else {
        return Ok((None, Config::default()));
    };
    let source = std::fs::read_to_string(&path).map_err(|err| InputError::at(&path, err))?;
    let config = Config::parse(&path, &source)?;
    Ok((Some(path), config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn read(source: &str) -> Result<Config, crate::InputError> {
        Config::parse(Path::new("parent/.crapts.toml"), source)
    }

    #[test]
    fn canonical_keys_and_the_three_aliases_resolve_to_the_same_settings() {
        let source = r#"
threshold = -15
fail-above = false
missing = "skip"
exclude = ["skip/**"]
default-excludes = []
allow = ["a", "b"]
min = 0.5
top = 9007199254740991
sort = "file"
epsilon = 1e-3
fail-regression = true
show-unchanged = true
fail-changed = false
uncovered-hints = false
"#;
        let config = read(source).unwrap();
        let aliases = source
            .replace("default-excludes", "default_excludes")
            .replace("show-unchanged", "show_unchanged")
            .replace("uncovered-hints", "uncovered_hints");
        assert_eq!(config, read(&aliases).unwrap());
        assert_eq!(config.threshold, Some(-15.0));
        assert_eq!(config.min, Some(0.5));
        assert_eq!(config.epsilon, Some(0.001));
        assert_eq!(config.top, Some(9_007_199_254_740_991));
        assert_eq!(config.missing, Some(MissingCoveragePolicy::Skip));
        assert_eq!(config.sort, Some(SortOrder::File));
        assert_eq!(config.exclude, Some(vec!["skip/**".to_owned()]));
        assert_eq!(config.allow, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(config.default_excludes, Some(vec![]));
        assert_eq!(config.fail_above, Some(false));
        assert_eq!(config.fail_regression, Some(true));
        assert_eq!(config.show_unchanged, Some(true));
        assert_eq!(config.fail_changed, Some(false));
        assert_eq!(config.uncovered_hints, Some(false));
    }

    #[test]
    fn config_reports_type_spelling_and_syntax_errors_at_their_source() {
        for key in [
            "thresold",
            "fail_above",
            "fail_regression",
            "fail_changed",
            "coverage",
            "format",
            "output",
            "base",
            "all-rows",
            "paths",
            "repo-url",
            "commit-ref",
            "comment-marker",
            "summary",
            "baseline",
        ] {
            let error = read(&format!("{key} = true")).unwrap_err();
            assert!(
                error.to_string().contains(&format!("unknown key {key}")),
                "{error}"
            );
            assert_eq!(error.path(), Path::new("parent/.crapts.toml"));
        }
        for (source, detail) in [
            ("allow = [3]", "allow"),
            ("fail-above = 1", "fail-above"),
            ("threshold = '15'", "threshold"),
            ("top = 1.0", "top"),
            ("top = -1", "top"),
            ("missing = 'maybe'", "maybe"),
            ("sort = 'size'", "size"),
            ("min = nan", "min must be finite"),
            ("threshold = inf", "threshold must be finite"),
            ("epsilon = -inf", "epsilon must be finite"),
            ("epsilon = -0.25", "epsilon must be at least 0"),
            ("threshold = 15\nallow = [", "line 2, column"),
        ] {
            let error = read(source).unwrap_err();
            assert_eq!(error.path(), Path::new("parent/.crapts.toml"));
            assert!(error.to_string().contains("parent/.crapts.toml"));
            assert!(error.to_string().contains(detail), "{source}: {error}");
        }
        for (key, val) in [
            ("default-excludes", "[]"),
            ("show-unchanged", "true"),
            ("uncovered-hints", "false"),
        ] {
            let aliases = format!("{key}={val}\n{}={val}", key.replace('-', "_"));
            assert!(
                read(&aliases)
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("duplicate {key}"))
            );
        }
    }

    #[test]
    fn zero_is_an_inclusive_epsilon_bound_and_top_includes_both_endpoints() {
        for top in [1, MAX_TOP] {
            let config = read(&format!("top={top}\nepsilon=0")).unwrap();
            assert_eq!(config.epsilon, Some(0.0));
            assert_eq!(config.top, Some(top));
        }
        for top in [0, MAX_TOP + 1] {
            let error = read(&format!("top={top}")).unwrap_err().to_string();
            assert!(error.contains(&top.to_string()) && error.contains("top"));
        }
    }

    #[test]
    fn missing_and_sort_keys_cover_all_their_wire_spellings() {
        for (key, value) in [
            ("pessimistic", MissingCoveragePolicy::Pessimistic),
            ("optimistic", MissingCoveragePolicy::Optimistic),
            ("skip", MissingCoveragePolicy::Skip),
        ] {
            assert_eq!(
                read(&format!("missing='{key}'")).unwrap().missing,
                Some(value)
            );
        }
        for (key, value) in [("crap", SortOrder::Crap), ("file", SortOrder::File)] {
            assert_eq!(read(&format!("sort='{key}'")).unwrap().sort, Some(value));
        }
    }

    proptest! {
        #[test]
        fn finite_min_and_threshold_values_round_trip(value in any::<f64>().prop_filter("finite numeric settings", |f| f.is_finite())) {
            let source = format!("threshold = {value:e}\nmin = {value:e}");
            let cfg = read(&source).unwrap();
            prop_assert_eq!(cfg.min.unwrap().to_bits(), value.to_bits());
            prop_assert_eq!(cfg.threshold.unwrap().to_bits(), value.to_bits());
            let epsilon=read(&format!("epsilon={value:e}"));
            if value < 0.0 {
                prop_assert!(epsilon.is_err());
            } else {
                prop_assert_eq!(epsilon.unwrap().epsilon.unwrap().to_bits(),value.to_bits());
            }
        }

        #[test]
        fn top_accepts_exactly_the_positive_safe_integers(value in prop_oneof![0i64..4, 9_007_199_254_740_987..9_007_199_254_741_000i64, any::<i64>()]) {
            prop_assert_eq!(read(&format!("top = {value}")).is_ok(), (1..=9_007_199_254_740_991).contains(&value));
        }
    }
}
