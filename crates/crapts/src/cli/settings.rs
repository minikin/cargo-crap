use super::{ArgMatches, Choice, Config, Format, MissingCoveragePolicy, SortOrder};
use super::{InputError, Result};
use crate::{DiscoveryOptions, ExcludePattern, GlobOrigin};
use cargo_crap_core::filter::AllowSet;
use serde::{Serialize, Serializer};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Origin {
    Default,
    Config,
    Flag,
}

#[derive(Debug, Serialize)]
pub(super) struct Setting<T> {
    pub value: T,
    pub origin: Origin,
}
impl<T> Setting<T> {
    fn pick(
        default: T,
        config: Option<T>,
        flag: Option<T>,
    ) -> Self {
        let (value, origin) = if let Some(v) = flag {
            (v, Origin::Flag)
        } else if let Some(v) = config {
            (v, Origin::Config)
        } else {
            (default, Origin::Default)
        };
        Self { value, origin }
    }
}
fn last<T: Clone + Send + Sync + 'static>(
    args: &ArgMatches,
    key: &str,
) -> Option<T> {
    args.get_many::<T>(key)?.next_back().cloned()
}
fn scalar<T: Clone + Send + Sync + 'static>(
    args: &ArgMatches,
    key: &str,
    default: T,
    config: Option<T>,
) -> Setting<T> {
    Setting::pick(default, config, last(args, key))
}
fn optional<T: Clone + Send + Sync + 'static>(
    args: &ArgMatches,
    key: &str,
    config: Option<T>,
    default: Option<T>,
) -> Setting<Option<T>> {
    Setting::pick(default, config.map(Some), last::<T>(args, key).map(Some))
}
fn switch(
    args: &ArgMatches,
    key: &str,
    config: Option<bool>,
) -> Setting<bool> {
    Setting::pick(false, config, args.get_flag(key).then_some(true))
}
fn choice<T: Choice, S: Serializer>(
    setting: &Setting<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    Setting {
        value: setting.value.label(),
        origin: setting.origin,
    }
    .serialize(serializer)
}

#[derive(Serialize)]
struct PrintedGlob<'a> {
    pattern: &'a str,
    origin: Origin,
}
fn globs<S: Serializer>(
    setting: &Setting<Vec<ExcludePattern>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let value = setting
        .value
        .iter()
        .map(|p| PrintedGlob {
            pattern: &p.pattern,
            origin: match &p.origin {
                GlobOrigin::Default => Origin::Default,
                GlobOrigin::Config(_) => Origin::Config,
                GlobOrigin::Flag => Origin::Flag,
            },
        })
        .collect::<Vec<_>>();
    Setting {
        value,
        origin: setting.origin,
    }
    .serialize(serializer)
}

#[derive(Debug, Serialize)]
pub(super) struct Effective {
    #[serde(skip)]
    pub allow_set: AllowSet,
    pub paths: Setting<Vec<PathBuf>>,
    pub coverage: Setting<Option<PathBuf>>,
    pub threshold: Setting<f64>,
    pub fail_above: Setting<bool>,
    pub baseline: Setting<Option<PathBuf>>,
    pub fail_regression: Setting<bool>,
    pub epsilon: Setting<f64>,
    pub show_unchanged: Setting<bool>,
    pub base: Setting<Option<String>>,
    pub fail_changed: Setting<bool>,
    #[serde(serialize_with = "choice")]
    pub missing: Setting<MissingCoveragePolicy>,
    #[serde(serialize_with = "globs")]
    pub allow: Setting<Vec<ExcludePattern>>,
    #[serde(serialize_with = "globs")]
    pub exclude: Setting<Vec<ExcludePattern>>,
    #[serde(serialize_with = "globs")]
    pub default_excludes: Setting<Vec<ExcludePattern>>,
    pub min: Setting<Option<f64>>,
    pub top: Setting<Option<u64>>,
    #[serde(serialize_with = "choice")]
    pub sort: Setting<SortOrder>,
    pub summary: Setting<bool>,
    #[serde(serialize_with = "choice")]
    pub format: Setting<Format>,
    pub output: Setting<Option<PathBuf>>,
    pub repo_url: Setting<Option<String>>,
    pub commit_ref: Setting<Option<String>>,
    pub comment_marker: Setting<String>,
    pub uncovered_hints: Setting<bool>,
}

fn rules(
    patterns: impl IntoIterator<Item = String>,
    origin: &GlobOrigin,
    base: &Path,
) -> Vec<ExcludePattern> {
    patterns
        .into_iter()
        .map(|pattern| ExcludePattern {
            pattern,
            origin: origin.clone(),
            base_dir: base.to_owned(),
        })
        .collect()
}

struct Bases<'a> {
    cwd: &'a Path,
    config: Option<&'a Path>,
}
impl Bases<'_> {
    fn configured(
        &self,
        patterns: Option<Vec<String>>,
    ) -> Option<Vec<ExcludePattern>> {
        let patterns = patterns?;
        let path = self
            .config
            .expect("config values come from a selected file");
        Some(rules(
            patterns,
            &GlobOrigin::Config(path.to_owned()),
            path.parent().unwrap_or(self.cwd),
        ))
    }

    fn list(
        &self,
        args: &ArgMatches,
        key: &str,
        config: Option<Vec<String>>,
    ) -> Setting<Vec<ExcludePattern>> {
        let mut set = Setting::pick(Vec::new(), self.configured(config), None);
        if let Some(flags) = args.get_many::<String>(key) {
            set.value
                .extend(rules(flags.cloned(), &GlobOrigin::Flag, self.cwd));
            set.origin = Origin::Flag;
        }
        set
    }

    fn defaults(
        &self,
        config: Option<Vec<String>>,
        clear: bool,
    ) -> Result<Setting<Vec<ExcludePattern>>> {
        let defaults = DiscoveryOptions::new(self.cwd).defaults;
        let configured = self.configured(config);
        let mut set = Setting::pick(defaults, configured, None);
        DiscoveryOptions::validate(self.cwd, &set.value)?;
        if clear {
            set.value.clear();
            set.origin = Origin::Flag;
        }
        Ok(set)
    }
}

fn compile_allow(
    cwd: &Path,
    rules: &[ExcludePattern],
) -> Result<AllowSet> {
    let pairs = rules
        .iter()
        .map(|p| (p.pattern.as_str(), p.base_dir.as_path()))
        .collect::<Vec<_>>();
    AllowSet::compile_with_bases(cwd, &pairs)
        .map_err(|error| InputError::message(format!("{error:#}")))
}

fn validate_allow(
    cwd: &Path,
    path: Option<&Path>,
    rules: &[ExcludePattern],
) -> Result<AllowSet> {
    compile_allow(cwd, rules).or_else(|error| {
        if let Some(path) = path {
            let count = rules.partition_point(|rule| matches!(rule.origin, GlobOrigin::Config(_)));
            compile_allow(cwd, &rules[..count]).map_err(|error| InputError::at(path, error))?;
        }
        Err(error)
    })
}

fn repo_from_environment() -> Option<String> {
    let server = std::env::var("GITHUB_SERVER_URL").ok()?;
    let repo = std::env::var("GITHUB_REPOSITORY").ok()?;
    Some(format!("{}/{repo}", server.trim_end_matches('/')))
}

impl Effective {
    pub(super) fn resolve(
        cwd: &Path,
        path: Option<&Path>,
        config: Config,
        args: &ArgMatches,
    ) -> Result<Self> {
        let bases = Bases { cwd, config: path };
        let defaults = bases.defaults(
            config.default_excludes,
            args.get_flag("no-default-excludes"),
        )?;
        let exclude = bases.list(args, "exclude", config.exclude);
        DiscoveryOptions::validate(cwd, &exclude.value)?;
        let allow = bases.list(args, "allow", config.allow);
        let allow_set = validate_allow(cwd, path, &allow.value)?;
        let mut min = optional(args, "min", config.min, None);
        let mut top = optional(args, "top", config.top, None);
        if args.get_flag("all-rows") {
            min.value = None;
            top.value = None;
        }
        let paths = args
            .get_many::<PathBuf>("paths")
            .map(|paths| paths.cloned().collect());
        Ok(Self {
            paths: Setting::pick(vec![PathBuf::from(".")], None, paths),
            coverage: optional(args, "coverage", None, None),
            threshold: scalar(
                args,
                "threshold",
                cargo_crap_core::score::DEFAULT_THRESHOLD,
                config.threshold,
            ),
            fail_above: switch(args, "fail-above", config.fail_above),
            baseline: optional(args, "baseline", None, None),
            fail_regression: switch(args, "fail-regression", config.fail_regression),
            epsilon: scalar(
                args,
                "epsilon",
                cargo_crap_core::delta::DEFAULT_EPSILON,
                config.epsilon,
            ),
            show_unchanged: switch(args, "show-unchanged", config.show_unchanged),
            base: optional(args, "base", None, None),
            fail_changed: switch(args, "fail-changed", config.fail_changed),
            missing: scalar(
                args,
                "missing",
                MissingCoveragePolicy::Pessimistic,
                config.missing,
            ),
            allow_set,
            allow,
            exclude,
            default_excludes: defaults,
            min,
            top,
            sort: scalar(args, "sort", SortOrder::Crap, config.sort),
            summary: switch(args, "summary", None),
            format: scalar(args, "format", Format::Human, None),
            output: optional(args, "output", None, None),
            repo_url: optional(args, "repo-url", None, repo_from_environment()),
            commit_ref: optional(args, "commit-ref", None, std::env::var("GITHUB_SHA").ok()),
            comment_marker: scalar(
                args,
                "comment-marker",
                "<!-- crapts-report -->".into(),
                None,
            ),
            uncovered_hints: Setting::pick(false, config.uncovered_hints, None),
        })
    }

    pub(super) fn discovery(
        &self,
        cwd: &Path,
    ) -> DiscoveryOptions {
        DiscoveryOptions {
            cwd: cwd.to_path_buf(),
            defaults: self.default_excludes.value.clone(),
            excludes: self.exclude.value.clone(),
        }
    }
}
