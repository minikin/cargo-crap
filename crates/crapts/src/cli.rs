use crate::InputError;
use crate::config::{self, Config, MAX_TOP};

type Result<T, E = InputError> = std::result::Result<T, E>;
use cargo_crap_core::entry::{MissingCoveragePolicy, SortOrder};
use cargo_crap_core::report::Format;
use clap::{Arg, ArgAction, ArgMatches, Command, builder::ValueParser};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod scoring;
mod settings;
use settings::Effective;

const VALUES: &[(&str, &str, &str)] = &[
    (
        "coverage",
        "file",
        "Read Istanbul or LCOV coverage from this file",
    ),
    (
        "threshold",
        "N",
        "Count scores strictly above this threshold",
    ),
    (
        "baseline",
        "file",
        "Select a baseline report for comparison",
    ),
    ("epsilon", "N", "Tolerance for comparing scores"),
    ("base", "revision", "Select a Git base for comparison"),
    (
        "missing",
        "policy",
        "Handle missing coverage as pessimistic, optimistic, or skip",
    ),
    ("allow", "glob", "Exempt matching functions or paths"),
    ("exclude", "glob", "Exclude matching source paths"),
    ("min", "score", "Display scores at or above this value"),
    ("top", "N", "Display the N worst eligible rows"),
    ("sort", "order", "Order displayed rows by crap or file"),
    (
        "format",
        "format",
        "Report as text (alias human), json, markdown, pr-comment, github, sarif, or shields",
    ),
    ("output", "file", "Write the report to this file"),
    (
        "repo-url",
        "url",
        "Use this repository URL for source links",
    ),
    ("commit-ref", "ref", "Use this revision for source links"),
    (
        "comment-marker",
        "text",
        "Set the first line of a PR comment",
    ),
];
const SWITCHES: &[(&str, &str)] = &[
    (
        "fail-above",
        "Exit 1 when an eligible score exceeds the threshold",
    ),
    ("fail-regression", "Gate regressions against the baseline"),
    ("show-unchanged", "Show unchanged baseline rows"),
    (
        "fail-changed",
        "Gate changed functions against the Git base",
    ),
    (
        "no-default-excludes",
        "Clear the default exclusion patterns",
    ),
    ("summary", "Summarize the displayed rows"),
    ("all-rows", "Clear both display limits"),
    ("print-config", "Print effective settings and their origins"),
];

trait Choice: Copy + Send + Sync + 'static {
    const VALUES: &'static [(&'static str, Self)];
    fn parse(
        name: &'static str,
        value: &str,
    ) -> Result<Self, String> {
        Self::VALUES
            .iter()
            .find(|(label, _)| *label == value)
            .map(|&(_, item)| item)
            .ok_or_else(|| format!("unknown --{name} {value}"))
    }
    fn label(self) -> &'static str;
}
impl Choice for Format {
    const VALUES: &'static [(&'static str, Self)] = &[
        ("text", Self::Human),
        ("human", Self::Human),
        ("json", Self::Json),
        ("markdown", Self::Markdown),
        ("pr-comment", Self::PrComment),
        ("github", Self::GitHub),
        ("sarif", Self::Sarif),
        ("shields", Self::Shields),
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Human => "text",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::PrComment => "pr-comment",
            Self::GitHub => "github",
            Self::Sarif => "sarif",
            Self::Shields => "shields",
        }
    }
}
impl Choice for SortOrder {
    const VALUES: &'static [(&'static str, Self)] = &[("crap", Self::Crap), ("file", Self::File)];
    fn label(self) -> &'static str {
        match self {
            Self::Crap => "crap",
            Self::File => "file",
        }
    }
}
impl Choice for MissingCoveragePolicy {
    const VALUES: &'static [(&'static str, Self)] = &[
        ("pessimistic", Self::Pessimistic),
        ("optimistic", Self::Optimistic),
        ("skip", Self::Skip),
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Pessimistic => "pessimistic",
            Self::Optimistic => "optimistic",
            Self::Skip => "skip",
        }
    }
}

fn number(
    name: &str,
    value: &str,
) -> Result<f64, String> {
    let n = value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("--{name} must be a finite number, got {value}"))?;
    if name == "epsilon" && n < 0.0 {
        return Err(format!("--epsilon must be at least 0, got {value}"));
    }
    Ok(n)
}
fn top(value: &str) -> Result<u64, String> {
    value
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .filter(|n| (1..=MAX_TOP).contains(n))
        .ok_or_else(|| format!("--top must be an integer of at least 1, got {value}"))
}
fn marker(value: &str) -> Result<String, String> {
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err("--comment-marker must be one nonempty line".into());
    }
    Ok(value.to_owned())
}

fn value_parser(name: &'static str) -> ValueParser {
    match name {
        "min" | "threshold" | "epsilon" => ValueParser::new(move |v: &str| number(name, v)),
        "top" => ValueParser::new(top),
        "format" => ValueParser::new(|v: &str| Format::parse("format", v)),
        "sort" => ValueParser::new(|v: &str| SortOrder::parse("sort", v)),
        "missing" => ValueParser::new(|v: &str| MissingCoveragePolicy::parse("missing", v)),
        "comment-marker" => ValueParser::new(marker),
        "coverage" | "baseline" | "output" => clap::value_parser!(PathBuf),
        _ => ValueParser::string(),
    }
}

fn command() -> Command {
    let mut cmd = Command::new("crapts")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .no_binary_name(true)
        .args_override_self(true)
        .color(clap::ColorChoice::Never)
        .override_usage("crapts [paths...] --coverage <file> [options]")
        .about("Score JavaScript and TypeScript functions using cyclomatic complexity and test coverage.")
        .arg(
            Arg::new("paths")
                .value_name("paths")
                .action(ArgAction::Append)
                .value_parser(clap::value_parser!(PathBuf)),
        );
    for &(name, label, help) in VALUES {
        cmd = cmd.arg(
            Arg::new(name)
                .long(name)
                .value_name(label)
                .action(ArgAction::Append)
                .value_parser(value_parser(name))
                .help(help),
        );
    }
    for &(name, help) in SWITCHES {
        cmd = cmd.arg(
            Arg::new(name)
                .long(name)
                .action(ArgAction::SetTrue)
                .help(help),
        );
    }
    for (long, short, help) in [
        ("help", 'h', "Print help"),
        ("version", 'V', "Print the package version"),
    ] {
        cmd = cmd.arg(
            Arg::new(long)
                .long(long)
                .short(short)
                .action(ArgAction::SetTrue)
                .help(help),
        );
    }
    cmd
}

#[derive(PartialEq)]
enum Route {
    Help,
    Version,
    Run,
}
fn short_info(token: &str) -> Option<&str> {
    token
        .strip_prefix('-')
        .filter(|s| !s.is_empty() && s.bytes().all(|b| matches!(b, b'h' | b'V')))
}
fn priority(args: &[OsString]) -> Route {
    let mut route = Route::Run;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            break;
        }
        let Some(token) = word.to_str() else {
            continue;
        };
        let short = short_info(token);
        if token == "--help" || short.is_some_and(|s| s.contains('h')) {
            return Route::Help;
        }
        if token == "--version" || short.is_some() {
            route = Route::Version;
        }
        if token
            .strip_prefix("--")
            .is_some_and(|k| VALUES.iter().any(|&(name, _, _)| k == name))
        {
            words.next();
        }
    }
    route
}

fn argument_value<'a>(
    name: &str,
    next: Option<&'a OsString>,
) -> Result<&'a OsString> {
    let Some(value) = next else {
        return Err(InputError::message(format!("missing value for --{name}")));
    };
    if let Some(text) = value.to_str()
        && text.starts_with('-')
        && text != "-"
    {
        let signed =
            matches!(name, "min" | "threshold" | "epsilon" | "top") && !text.starts_with("--");
        if !signed {
            return Err(InputError::message(format!(
                "ambiguous argument for --{name}: {text}"
            )));
        }
    }
    Ok(value)
}
fn normalize_flag<'a>(
    flag: &str,
    words: &mut impl Iterator<Item = &'a OsString>,
) -> Result<OsString> {
    let (key, attached) = flag
        .split_once('=')
        .map_or((flag, false), |(key, _)| (key, true));
    let name = key.strip_prefix("--").unwrap_or(key);
    if VALUES.iter().any(|&(k, _, _)| k == name) {
        if !attached {
            let mut token = OsString::from(key);
            token.push("=");
            token.push(argument_value(name, words.next())?);
            return Ok(token);
        }
    } else {
        if !matches!(name, "help" | "version") && !SWITCHES.iter().any(|&(k, _)| k == name) {
            return Err(InputError::message(format!("unknown option {key}")));
        }
        if attached {
            return Err(InputError::message(format!(
                "{key} does not take an argument"
            )));
        }
    }
    Ok(flag.into())
}

fn normalized_args(args: &[OsString]) -> Result<Vec<OsString>> {
    let mut result = Vec::with_capacity(args.len());
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            result.push(word.clone());
            result.extend(words.cloned());
            break;
        }
        let token = match word.to_str() {
            Some(flag) if flag.starts_with('-') && flag != "-" => normalize_flag(flag, &mut words)?,
            _ => word.clone(),
        };
        result.push(token);
    }
    Ok(result)
}

fn parse(args: &[OsString]) -> Result<ArgMatches> {
    command()
        .try_get_matches_from(normalized_args(args)?)
        .map_err(|error| clap_error(&error))
}

fn clap_error(error: &clap::Error) -> InputError {
    if error.kind() == clap::error::ErrorKind::ValueValidation
        && let Some(cause) = std::error::Error::source(error)
    {
        return InputError::message(cause);
    }
    let rendered = error.render().to_string();
    let message = rendered.strip_prefix("error: ").unwrap_or(&rendered);
    InputError::message(message.trim_end())
}

#[derive(Clone, Copy)]
struct StdoutSink {
    terminal: bool,
    width: Option<usize>,
}

#[derive(serde::Serialize)]
struct Printed<'a> {
    config_path: Option<&'a Path>,
    settings: &'a Effective,
}

fn stdout_error(error: impl std::fmt::Display) -> InputError {
    InputError::at(Path::new("stdout"), error)
}

fn finish_stdout(out: &mut dyn Write) -> Result<ExitCode> {
    out.flush().map_err(stdout_error)?;
    Ok(ExitCode::SUCCESS)
}

fn validate_comparisons(
    args: &ArgMatches,
    settings: &Effective,
) -> Result<()> {
    let baseline = settings.baseline.value.is_some();
    let base = settings.base.value.is_some();
    for (flag, selector, selected) in [
        ("fail-regression", "baseline", baseline),
        ("show-unchanged", "baseline", baseline),
        ("fail-changed", "base", base),
    ] {
        if args.get_flag(flag) && !selected {
            return Err(InputError::message(format!("--{flag} needs --{selector}")));
        }
    }
    if let Some((selector, _)) = [("baseline", baseline), ("base", base)]
        .into_iter()
        .find(|(_, selected)| *selected)
    {
        if settings.format.value == Format::Sarif {
            return Err(InputError::message(format!(
                "SARIF does not support --{selector}"
            )));
        }
        if selector == "base" {
            return Err(InputError::message("--base is not implemented yet"));
        }
    }
    Ok(())
}

fn configured(
    cwd: &Path,
    args: &[OsString],
    sink: StdoutSink,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<ExitCode> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| InputError::at(cwd, error))?;
    let worktree = crate::discover::Worktree::probe(&cwd)?;
    let config = config::load_at(&cwd, worktree.root.as_deref())?;
    let args = parse(args)?;
    let settings = Effective::resolve(&cwd, config, &args)?;
    if args.get_flag("print-config") {
        let json = serde_json::to_string_pretty(&Printed {
            config_path: settings.config_path.as_deref(),
            settings: &settings,
        })
        .map_err(InputError::message)?;
        writeln!(out, "{json}").map_err(stdout_error)?;
        return finish_stdout(out);
    }
    validate_comparisons(&args, &settings)?;
    scoring::score(&cwd, worktree, &settings, sink, out, err)
}

fn execute(
    cwd: &Path,
    args: &[OsString],
    sink: StdoutSink,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<ExitCode> {
    match priority(args) {
        Route::Help => write!(out, "{}", command().render_long_help()).map_err(stdout_error)?,
        Route::Version => {
            writeln!(out, "crapts {}", env!("CARGO_PKG_VERSION")).map_err(stdout_error)?;
        },
        Route::Run => return configured(cwd, args, sink, out, err),
    }
    finish_stdout(out)
}

pub fn run(
    cwd: &Path,
    args: &[OsString],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> ExitCode {
    run_with_stdout(
        cwd,
        args,
        out,
        err,
        false,
        cargo_crap_core::report::output_width(false),
    )
}

#[doc(hidden)]
pub fn run_with_stdout(
    cwd: &Path,
    args: &[OsString],
    out: &mut dyn Write,
    err: &mut dyn Write,
    terminal: bool,
    width: Option<usize>,
) -> ExitCode {
    match execute(cwd, args, StdoutSink { terminal, width }, out, err) {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(err, "error: {error:#}");
            ExitCode::from(2)
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    fn resolved(
        config: Config,
        args: &[&str],
    ) -> Value {
        let cwd = std::env::current_dir().unwrap();
        let config_path = cwd.join(".crapts.toml");
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let parsed = parse(&args).unwrap();
        let settings = Effective::resolve(&cwd, Some((config_path, config)), &parsed).unwrap();
        serde_json::to_value(settings).unwrap()
    }

    #[test]
    fn print_config_uses_the_canonical_label_for_every_accepted_choice() {
        for &(spelling, _) in Format::VALUES {
            let settings = resolved(Config::default(), &["--format", spelling]);
            let canonical = if spelling == "human" {
                "text"
            } else {
                spelling
            };
            assert_eq!(
                settings["format"],
                json!({"value": canonical, "origin": "flag"})
            );
        }
        for &(spelling, _) in SortOrder::VALUES {
            let settings = resolved(Config::default(), &["--sort", spelling]);
            assert_eq!(settings["sort"]["value"], spelling);
        }
        for &(spelling, _) in MissingCoveragePolicy::VALUES {
            let settings = resolved(Config::default(), &["--missing", spelling]);
            assert_eq!(settings["missing"]["value"], spelling);
        }
    }

    #[test]
    fn clap_errors_without_a_parser_cause_use_the_rendered_message() {
        for kind in [
            clap::error::ErrorKind::InvalidValue,
            clap::error::ErrorKind::ValueValidation,
        ] {
            let error = clap::Error::raw(kind, "invalid argument\n\n");
            assert_eq!(clap_error(&error).to_string(), "invalid argument");
        }
    }

    #[test]
    fn config_and_flags_retain_their_explicit_origins_and_glob_order() {
        let config = Config {
            allow: Some(vec!["one".into()]),
            exclude: Some(vec![]),
            default_excludes: Some(vec!["dist/**".into()]),
            fail_above: Some(false),
            ..Config::default()
        };
        let s = resolved(
            config,
            &[
                "--allow",
                "two",
                "--allow",
                "three",
                "--no-default-excludes",
            ],
        );
        assert_eq!(s["exclude"], json!({"value":[],"origin":"config"}));
        assert_eq!(s["fail_above"], json!({"value":false,"origin":"config"}));
        assert_eq!(s["default_excludes"], json!({"value":[],"origin":"flag"}));
        assert_eq!(
            s["allow"],
            json!({"value":[{"pattern":"one","origin":"config"},{"pattern":"two","origin":"flag"},{"pattern":"three","origin":"flag"}],"origin":"flag"})
        );
    }

    #[test]
    fn every_scalar_occurrence_and_every_config_rule_is_validated() {
        let d = tempfile::tempdir().unwrap();
        for flags in [
            ["--threshold", "nope", "--threshold", "12"],
            ["--missing", "bad", "--missing", "skip"],
            ["--format", "no", "--format", "json"],
            ["--top", "0", "--top", "2"],
        ] {
            let args = flags.map(OsString::from);
            assert!(parse(&args).is_err(), "{flags:?}");
        }
        let path = d.path().join(".crapts.toml");
        for key in ["default-excludes", "exclude", "allow"] {
            let config = Config::parse(&path, &format!("{key} = ['src/[']")).unwrap();
            let parsed = parse(&["--no-default-excludes".into()]).unwrap();
            let error =
                Effective::resolve(d.path(), Some((path.clone(), config)), &parsed).unwrap_err();
            let err = format!("{error:#}");
            assert!(
                err.contains("src/[") && err.contains(".crapts.toml"),
                "{err}"
            );
        }
    }

    #[test]
    fn every_information_action_checks_stdout_including_flush() {
        struct Broken {
            flush_only: bool,
        }
        impl std::io::Write for Broken {
            fn write(
                &mut self,
                b: &[u8],
            ) -> std::io::Result<usize> {
                if self.flush_only {
                    Ok(b.len())
                } else {
                    Err(std::io::Error::other("injected write"))
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("injected flush"))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        for flag in ["--help", "--version", "--print-config"] {
            for flush_only in [true, false] {
                let mut err = Vec::new();
                assert_eq!(
                    run(
                        dir.path(),
                        &[flag.into()],
                        &mut Broken { flush_only },
                        &mut err
                    ),
                    ExitCode::from(2)
                );
                let err = String::from_utf8(err).unwrap();
                assert!(
                    err.contains("stdout")
                        && err.contains(if flush_only {
                            "injected flush"
                        } else {
                            "injected write"
                        }),
                    "{err}"
                );
            }
        }
    }

    proptest! {
        #[test]
        fn scalar_last_wins(values in prop::collection::vec(-3000i32..3000,1..6)) {
            let flags:Vec<_>=values.iter().map(|v|format!("--threshold={v}")).collect();
            let refs:Vec<_>=flags.iter().map(String::as_str).collect();
            let printed=resolved(Config{threshold:Some(30.),..Config::default()},&refs);
            prop_assert_eq!(printed["threshold"]["value"].as_f64(),Some(f64::from(*values.last().unwrap())));
            prop_assert_eq!(&printed["threshold"]["origin"],"flag");
        }

        #[test]
        fn clearing_a_limit_keeps_the_highest_explicit_source(
            cfg_min in prop::option::of(-9i32..10), cfg_top in prop::option::of(1u64..10),
            flag_min in prop::option::of(-9i32..10), flag_top in prop::option::of(1u32..10)
        ) {
            let mut args=vec!["--all-rows".to_owned()];
            if let Some(value)=flag_min {args.push(format!("--min={value}"));}
            if let Some(value)=flag_top {args.push(format!("--top={value}"));}
            let refs:Vec<_>=args.iter().map(String::as_str).collect();
            let printed=resolved(Config{min:cfg_min.map(f64::from),top:cfg_top,..Config::default()},&refs);
            for (key,config,flag) in [("min",cfg_min.is_some(),flag_min.is_some()),("top",cfg_top.is_some(),flag_top.is_some())] {
                prop_assert!(printed[key]["value"].is_null());
                prop_assert_eq!(&printed[key]["origin"],if flag {"flag"} else if config {"config"} else {"default"});
            }
        }
    }
}

#[cfg(test)]
mod writer_tests {
    use super::*;
    struct RefusesFlush {
        bytes: Vec<u8>,
        attempted: bool,
    }
    impl std::io::Write for RefusesFlush {
        fn write(
            &mut self,
            buf: &[u8],
        ) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.attempted = true;
            Err(std::io::Error::other("injected flush failure"))
        }
    }

    #[test]
    fn an_explicit_companion_is_checked_before_discovery_or_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let args =
            ["absent.ts", "--coverage", "absent.info", "--show-unchanged"].map(OsString::from);
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            run(dir.path(), &args, &mut out, &mut err),
            ExitCode::from(2)
        );
        assert_eq!(out.as_slice(), b"");
        assert_eq!(err, b"error: --show-unchanged needs --baseline\n");
    }

    #[test]
    fn sarif_and_companion_checks_precede_every_baseline_and_coverage_read() {
        let dir = tempfile::tempdir().unwrap();
        for (extra, expected) in [
            (
                &["--format", "sarif", "--baseline", "absent.json"][..],
                "SARIF does not support --baseline",
            ),
            (
                &[
                    "--format",
                    "sarif",
                    "--baseline",
                    "absent.json",
                    "--fail-regression",
                ][..],
                "SARIF does not support --baseline",
            ),
            (
                &["--fail-regression"][..],
                "--fail-regression needs --baseline",
            ),
        ] {
            let mut args = vec![OsString::from("--coverage"), "absent.info".into()];
            args.extend(extra.iter().map(OsString::from));
            let mut out = Vec::new();
            let mut err = Vec::new();
            assert_eq!(
                run(dir.path(), &args, &mut out, &mut err),
                ExitCode::from(2)
            );
            assert_eq!(out.as_slice(), b"");
            assert_eq!(
                String::from_utf8(err).unwrap(),
                format!("error: {expected}\n")
            );
        }
    }

    #[test]
    fn a_flush_only_error_overrides_the_runners_verdict() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("f.ts"),
            "function hot(a,b) { if(a) return 1; if(b) return 2; return 0; }",
        )
        .unwrap();
        std::fs::write(d.path().join("cov"), "SF:f.ts\nDA:1,0\nend_of_record").unwrap();
        for threshold in ["6", "12"] {
            let mut sink = RefusesFlush {
                bytes: Vec::new(),
                attempted: false,
            };
            let mut err = Vec::new();
            let args = [
                "--coverage",
                "cov",
                "--format",
                "json",
                "--threshold",
                threshold,
                "--fail-above",
            ]
            .map(OsString::from);
            assert_eq!(
                run(d.path(), &args, &mut sink, &mut err),
                std::process::ExitCode::from(2)
            );
            assert!(sink.attempted);
            let doc: serde_json::Value = serde_json::from_slice(&sink.bytes).unwrap();
            assert_eq!(doc["summary"]["crappy"], usize::from(threshold == "6"));
            let diagnostic = String::from_utf8(err).unwrap();
            assert!(
                diagnostic.contains("stdout") && diagnostic.contains("injected flush failure"),
                "{diagnostic}"
            );
        }
    }
}
