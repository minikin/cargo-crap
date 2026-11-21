//! Checks on a case's raw output before it is compared or recorded: a
//! golden must not bless a wrong byte on its first recording.
//!
//! Which checks apply comes from the case's row, never from whether the
//! output happens to parse, so a broken report cannot slip past as text.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

use crate::encode::{Run, Status};
use crate::run::Case;
use crate::schemas;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PLACEHOLDER: &str = "[VERSION]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Report,
    Delta,
    Sarif,
    Text,
}

/// What the case's row says its stdout is. `--summary` turns SARIF into a
/// text line and leaves JSON a report, and a failed run prints no report.
pub fn kind(case: &Case) -> Kind {
    if case.exit > 1 {
        return Kind::Text;
    }
    let has = |flag: &str| case.args.contains(&flag);
    let format = case
        .args
        .iter()
        .skip_while(|arg| **arg != "--format")
        .nth(1)
        .copied();
    match format {
        Some("json") if has("--baseline") => Kind::Delta,
        Some("json") => Kind::Report,
        Some("sarif") if !has("--summary") => Kind::Sarif,
        _ => Kind::Text,
    }
}

pub fn finish(
    case: &Case,
    mut run: Run,
    root: &Path,
) -> Result<Run, Vec<String>> {
    let mut problems = leaked_paths(&run, root);
    if run.status != Status::Exit(case.exit) {
        problems.push(format!(
            "the run ended with {:?}, the Cases table says exit {}",
            run.status, case.exit
        ));
    }
    let kind = kind(case);
    if kind != Kind::Text {
        match json_checks(kind, &run.stdout) {
            Ok(stdout) => run.stdout = stdout.into_bytes(),
            Err(more) => problems.extend(more),
        }
    }
    if problems.is_empty() {
        Ok(run)
    } else {
        Err(problems)
    }
}

fn leaked_paths(
    run: &Run,
    root: &Path,
) -> Vec<String> {
    let places = [
        root.to_path_buf(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        std::env::temp_dir(),
    ];
    leaks(run, &forbidden(&places))
}

/// Each place as given and canonicalized: macOS hands out `/var/...` temp
/// dirs that a process may print as `/private/var/...`.
fn forbidden(places: &[PathBuf]) -> Vec<String> {
    let mut all: Vec<String> = places
        .iter()
        .flat_map(|place| [Some(place.clone()), place.canonicalize().ok()])
        .flatten()
        .map(|path| path.to_string_lossy().trim_end_matches('/').to_owned())
        .filter(|path| !path.is_empty())
        .collect();
    all.sort();
    all.dedup();
    all
}

fn leaks(
    run: &Run,
    forbidden: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    for (stream, bytes) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        for path in forbidden.iter().filter(|path| mentions(&text, path)) {
            problems.push(format!("{stream} names the path {path}"));
        }
    }
    problems
}

/// Whether `text` names `path` itself, not a longer path ending in it
/// (`x/tmp/`) or one it is a prefix of (`/tmpfoo`).
fn mentions(
    text: &str,
    path: &str,
) -> bool {
    text.match_indices(path).any(|(at, _)| {
        let head = &text[..at];
        let before = head.chars().next_back();
        // `file:///tmp/x` names /tmp although a slash precedes it.
        let starts = !before.is_some_and(is_path_char) || head.ends_with("file://");
        let after = text[at + path.len()..].chars().next();
        starts && after.is_none_or(|c| c == '/' || !is_path_char(c))
    })
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~')
}

fn json_checks(
    kind: Kind,
    stdout: &[u8],
) -> Result<String, Vec<String>> {
    let text =
        std::str::from_utf8(stdout).map_err(|e| vec![format!("stdout is not UTF-8: {e}")])?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| vec![format!("stdout is not JSON: {e}")])?;
    let mut problems = Vec::new();
    absolute_paths(&value, &mut problems);
    if let Some(schema) = schema(kind) {
        problems.extend(
            schema
                .validator()
                .iter_errors(&value)
                .map(|e| format!("does not validate against {}: {e}", schema.path)),
        );
    }
    match with_placeholder(kind, text, &value) {
        Ok(rewritten) if problems.is_empty() => Ok(rewritten),
        Ok(_) => Err(problems),
        Err(problem) => {
            problems.push(problem);
            Err(problems)
        },
    }
}

fn absolute_paths(
    value: &Value,
    problems: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let path_key = matches!(key.as_str(), "file" | "previous_file" | "uri");
                if let (true, Some(path)) = (path_key, inner.as_str())
                    && (path.starts_with('/') || has_scheme(path))
                {
                    problems.push(format!("\"{key}\" is absolute: {path}"));
                }
                absolute_paths(inner, problems);
            }
        },
        Value::Array(items) => items.iter().for_each(|item| absolute_paths(item, problems)),
        _ => {},
    }
}

/// A URI scheme such as `file:` or `https:`: an absolute location, where a
/// golden needs one relative to the case.
fn has_scheme(path: &str) -> bool {
    path.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

struct Schema {
    path: &'static str,
    compiled: &'static OnceLock<jsonschema::Validator>,
}

impl Schema {
    fn validator(&self) -> &jsonschema::Validator {
        self.compiled.get_or_init(|| schemas::compile(self.path))
    }
}

fn schema(kind: Kind) -> Option<Schema> {
    static REPORT: OnceLock<jsonschema::Validator> = OnceLock::new();
    static DELTA: OnceLock<jsonschema::Validator> = OnceLock::new();
    match kind {
        Kind::Report => Some(Schema {
            path: "schemas/report-v1.json",
            compiled: &REPORT,
        }),
        Kind::Delta => Some(Schema {
            path: "schemas/delta-v2.json",
            compiled: &DELTA,
        }),
        Kind::Sarif | Kind::Text => None,
    }
}

/// The text with the one field that carries cargo-crap's version rewritten,
/// checked by parsing it again: nothing but that field may change.
fn with_placeholder(
    kind: Kind,
    text: &str,
    value: &Value,
) -> Result<String, String> {
    let pointer = if kind == Kind::Sarif {
        "/runs/0/tool/driver/version"
    } else {
        "/version"
    };
    let found = value.pointer(pointer).and_then(Value::as_str);
    if found != Some(VERSION) {
        return Err(format!(
            "{pointer} is {found:?}, not cargo-crap's version {VERSION}"
        ));
    }
    let field = format!("\"version\": \"{VERSION}\"");
    if text.matches(&field).count() != 1 {
        return Err(format!(
            "{field} appears {} times, not once",
            text.matches(&field).count()
        ));
    }
    let rewritten = text.replacen(&field, &format!("\"version\": \"{PLACEHOLDER}\""), 1);
    let mut expected = value.clone();
    *expected.pointer_mut(pointer).expect("the field just read") = Value::from(PLACEHOLDER);
    let reparsed: Value = serde_json::from_str(&rewritten).map_err(|e| e.to_string())?;
    if reparsed == expected {
        Ok(rewritten)
    } else {
        Err(format!("rewriting {field} changed more than {pointer}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{Fixture, prepare, run_with};

    const P_JSON: Case = Case {
        id: "probe_p_json",
        fixture: Fixture::P,
        args: &["--format", "json"],
        config: None,
        env: &[],
        exit: 0,
    };

    const P_SARIF: Case = Case {
        id: "probe_p_sarif",
        args: &["--format", "sarif"],
        ..P_JSON
    };

    const P_HUMAN: Case = Case {
        id: "probe_p_human",
        args: &[],
        ..P_JSON
    };

    fn clean_host(key: &str) -> Option<std::ffi::OsString> {
        [
            "PATH",
            "HOME",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
        ]
        .contains(&key)
        .then(|| std::env::var_os(key))
        .flatten()
    }

    fn real(case: &Case) -> (Run, crate::run::CaseDir) {
        let copy = prepare(case);
        let run = run_with(case, &copy, &clean_host);
        assert_eq!(
            run.status,
            Status::Exit(0),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        (run, copy)
    }

    fn with_stdout(
        run: &Run,
        stdout: String,
    ) -> Run {
        Run {
            stdout: stdout.into_bytes(),
            ..run.clone()
        }
    }

    fn text(run: &Run) -> &str {
        std::str::from_utf8(&run.stdout).expect("utf-8")
    }

    fn fails_with(
        outcome: Result<Run, Vec<String>>,
        needle: &str,
    ) {
        let problems = outcome.expect_err("the checks passed");
        assert!(
            problems.iter().any(|p| p.contains(needle)),
            "no {needle:?} in {problems:?}"
        );
    }

    #[test]
    fn the_row_decides_the_kind() {
        let case = |args: &'static [&'static str], exit| Case {
            args,
            exit,
            ..P_JSON
        };
        assert_eq!(kind(&case(&["--format", "json"], 0)), Kind::Report);
        assert_eq!(
            kind(&case(&["--summary", "--format", "json"], 0)),
            Kind::Report
        );
        assert_eq!(
            kind(&case(&["--baseline", "b.json", "--format", "json"], 1)),
            Kind::Delta
        );
        assert_eq!(kind(&case(&["--format", "sarif"], 0)), Kind::Sarif);
        assert_eq!(
            kind(&case(&["--summary", "--format", "sarif"], 0)),
            Kind::Text
        );
        assert_eq!(
            kind(&case(&["--baseline", "b.json", "--format", "sarif"], 2)),
            Kind::Text
        );
        assert_eq!(kind(&case(&["--format", "markdown"], 0)), Kind::Text);
        assert_eq!(kind(&case(&[], 0)), Kind::Text);
        assert_eq!(
            kind(&case(&["--lcov", "missing.info", "--format", "json"], 2)),
            Kind::Text
        );
    }

    #[test]
    fn cargo_craps_version_is_recorded_as_version_and_only_there() {
        let (run, copy) = real(&P_JSON);
        let renamed = text(&run).replace(
            "\"function\": \"crappy\"",
            &format!("\"function\": \"{VERSION}\""),
        );
        let done = finish(&P_JSON, with_stdout(&run, renamed), copy.root()).expect("checks pass");
        assert!(
            text(&done).contains("\"version\": \"[VERSION]\""),
            "{}",
            text(&done)
        );
        assert!(text(&done).contains(&format!("\"function\": \"{VERSION}\"")));

        let (run, copy) = real(&P_SARIF);
        let done = finish(&P_SARIF, run, copy.root()).expect("checks pass");
        let sarif: Value = serde_json::from_slice(&done.stdout).expect("json");
        assert_eq!(
            sarif.pointer("/runs/0/tool/driver/version"),
            Some(&Value::from(PLACEHOLDER))
        );
        assert_eq!(sarif.pointer("/version"), Some(&Value::from("2.1.0")));
    }

    #[test]
    fn another_version_fails_the_case() {
        let (run, copy) = real(&P_JSON);
        let other = text(&run).replace(
            &format!("\"version\": \"{VERSION}\""),
            "\"version\": \"0.1.0\"",
        );
        fails_with(
            finish(&P_JSON, with_stdout(&run, other), copy.root()),
            "version",
        );
    }

    #[test]
    fn a_report_that_does_not_parse_fails_the_case() {
        let (run, copy) = real(&P_JSON);
        fails_with(
            finish(
                &P_JSON,
                with_stdout(&run, "✗ not json\n".into()),
                copy.root(),
            ),
            "JSON",
        );
    }

    #[test]
    fn text_output_passes_through_unchanged() {
        let (run, copy) = real(&P_HUMAN);
        assert_eq!(finish(&P_HUMAN, run.clone(), copy.root()), Ok(run));
    }

    #[test]
    fn an_absolute_path_in_output_fails_on_first_recording() {
        let (run, copy) = real(&P_JSON);
        let root = copy.root();
        let absolute = text(&run).replace(
            "\"file\": \"./src/lib.rs\"",
            "\"file\": \"/abs/src/lib.rs\"",
        );
        fails_with(
            finish(&P_JSON, with_stdout(&run, absolute), root),
            "absolute",
        );
        let (sarif, sarif_copy) = real(&P_SARIF);
        let uri = text(&sarif).replace("\"uri\": \"./src/lib.rs\"", "\"uri\": \"/abs/src/lib.rs\"");
        fails_with(
            finish(&P_SARIF, with_stdout(&sarif, uri), sarif_copy.root()),
            "absolute",
        );
        let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let canonical_root = root.canonicalize().expect("canonical root");
        let temp = std::env::temp_dir();
        for leak in [
            root,
            canonical_root.as_path(),
            checkout.as_path(),
            temp.as_path(),
        ] {
            let mut leaky = run.clone();
            leaky.stderr = format!("warning: {}/x.rs\n", leak.display()).into_bytes();
            fails_with(finish(&P_JSON, leaky, root), "names the path");
        }
    }

    #[test]
    fn a_run_whose_status_differs_from_its_row_fails() {
        let (run, copy) = real(&P_JSON);
        let row_says_two = Case { exit: 2, ..P_JSON };
        fails_with(
            finish(&row_says_two, run, copy.root()),
            "the Cases table says exit 2",
        );
        let missing = Case {
            id: "probe_p_missing_lcov",
            args: &["--lcov", "missing.info"],
            exit: 2,
            ..P_JSON
        };
        let copy = prepare(&missing);
        let run = run_with(&missing, &copy, &clean_host);
        assert_eq!(run.status, Status::Exit(2));
        assert!(finish(&missing, run, copy.root()).is_ok());
    }

    #[test]
    fn a_root_outside_the_temp_dir_and_a_path_ending_a_line_are_leaks() {
        let (run, _copy) = real(&P_HUMAN);
        let elsewhere = Path::new("/srv/golden-case-root");
        let mut leaky = run.clone();
        leaky.stderr = b"warning: /srv/golden-case-root/src/lib.rs\n".to_vec();
        fails_with(
            finish(&P_HUMAN, leaky, elsewhere),
            "names the path /srv/golden-case-root",
        );
        let mut at_end = run;
        at_end.stderr = b"analyzed /srv/golden-case-root\n".to_vec();
        fails_with(
            finish(&P_HUMAN, at_end, elsewhere),
            "names the path /srv/golden-case-root",
        );
    }

    #[test]
    fn a_path_counts_only_where_it_stands_alone() {
        assert!(mentions("warning: /repo/x.rs", "/repo"));
        assert!(mentions("at /repo: oops", "/repo"));
        assert!(mentions("`/repo`", "/repo"));
        assert!(mentions("\"/repo\"", "/repo"));
        assert!(mentions("/repo", "/repo"));
        assert!(!mentions("see x/tmp/y", "/tmp"));
        assert!(mentions("see file:///tmp/case/src/lib.rs", "/tmp"));
        assert!(!mentions("/tmpfoo/y", "/tmp"));
    }

    #[test]
    fn a_root_named_through_its_canonical_form_is_a_leak() {
        let base = tempfile::tempdir().expect("base");
        let real = base.path().join("real");
        std::fs::create_dir(&real).expect("real dir");
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let canonical = real.canonicalize().expect("canonical");
        let run = Run {
            stdout: Vec::new(),
            stderr: format!("warning: {}/x.rs\n", canonical.display()).into_bytes(),
            status: Status::Exit(0),
        };
        let as_given = vec![link.to_string_lossy().into_owned()];
        assert_eq!(leaks(&run, &as_given), Vec::<String>::new());
        assert_eq!(
            leaks(&run, &forbidden(&[link])),
            [format!("stderr names the path {}", canonical.display())]
        );
    }

    #[test]
    fn a_leak_on_stdout_fails_too() {
        let (mut run, copy) = real(&P_HUMAN);
        run.stdout
            .extend_from_slice(format!("{}/src/lib.rs\n", copy.root().display()).as_bytes());
        fails_with(finish(&P_HUMAN, run, copy.root()), "stdout names the path");
    }

    #[test]
    fn an_absolute_previous_file_is_rejected() {
        let delta: Value = serde_json::json!({
            "entries": [{ "file": "src/lib.rs", "previous_file": "/abs/old.rs" }],
        });
        let mut problems = Vec::new();
        absolute_paths(&delta, &mut problems);
        assert_eq!(problems, ["\"previous_file\" is absolute: /abs/old.rs"]);
        let sarif = serde_json::json!({ "uri": "https://example.org/src/lib.rs" });
        let mut problems = Vec::new();
        absolute_paths(&sarif, &mut problems);
        assert_eq!(
            problems,
            ["\"uri\" is absolute: https://example.org/src/lib.rs"]
        );
        let relative = serde_json::json!({ "uri": "./src/lib.rs", "file": "crates/a/src/lib.rs" });
        let mut problems = Vec::new();
        absolute_paths(&relative, &mut problems);
        assert_eq!(problems, Vec::<String>::new());
    }

    #[test]
    fn json_outputs_validate_against_the_unchanged_schemas() {
        let (run, copy) = real(&P_JSON);
        assert!(finish(&P_JSON, run.clone(), copy.root()).is_ok());
        let entries_gone = {
            let mut report: Value = serde_json::from_slice(&run.stdout).expect("json");
            report.as_object_mut().expect("object").remove("entries");
            serde_json::to_string_pretty(&report).expect("json")
        };
        fails_with(
            finish(&P_JSON, with_stdout(&run, entries_gone), copy.root()),
            "report-v1.json",
        );
        std::fs::write(copy.root().join("base.json"), &run.stdout).expect("baseline");
        let delta = Case {
            id: "probe_p_delta",
            args: &["--baseline", "base.json", "--format", "json"],
            ..P_JSON
        };
        let delta_run = run_with(&delta, &copy, &clean_host);
        assert!(finish(&delta, delta_run, copy.root()).is_ok());
        fails_with(finish(&delta, run, copy.root()), "delta-v2.json");
    }
}
