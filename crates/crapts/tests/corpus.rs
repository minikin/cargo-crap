mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use proptest::prelude::*;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

const PINNED_SHA: &str = "2c3dcc06ec807e81ab9437eceb1a8f7a3c590743";
const PUBLISHED_SCHEMAS: &str =
    "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/";
const VERSION_PLACEHOLDER: &str = "[VERSION]";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct LocalSchemas;

impl jsonschema::Retrieve for LocalSchemas {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let relative = uri
            .as_str()
            .strip_prefix(PUBLISHED_SCHEMAS)
            .ok_or_else(|| format!("not a published schema: {}", uri.as_str()))?;
        let path = workspace_root().join("schemas").join(relative);
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }
}

fn report_v1_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let path = workspace_root().join("schemas/crapts/report-v1.json");
        let raw =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let schema: Value = serde_json::from_str(&raw).expect("report-v1.json must be valid JSON");
        jsonschema::options()
            .with_retriever(LocalSchemas)
            .build(&schema)
            .expect("report-v1.json must compile offline")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorpusMode {
    Verify,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    MissingEnv,
    InvalidEnv,
    InvalidPin,
    MissingCorpusDir,
    GitCheck,
    PinMismatch,
    DirtyWorktree,
    Decompression,
    Command,
    InvalidOutput,
    PathLeak,
    Drift,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CorpusFailure {
    kind: FailureKind,
    message: String,
    added_rows: Vec<String>,
    removed_rows: Vec<String>,
}

impl CorpusFailure {
    fn new(
        kind: FailureKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            added_rows: Vec::new(),
            removed_rows: Vec::new(),
        }
    }
}

impl fmt::Display for CorpusFailure {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str(&self.message)
    }
}

struct LeakScope<'a> {
    corpus_dir: &'a Path,
    temp_coverage_path: &'a Path,
    home: Option<&'a Path>,
}

fn parse_corpus_dir(raw: Option<&str>) -> Result<PathBuf, CorpusFailure> {
    match raw {
        Some(value) if !value.trim().is_empty() => Ok(PathBuf::from(value)),
        _ => Err(CorpusFailure::new(
            FailureKind::MissingEnv,
            "CRAPTS_CORPUS_DIR must be set to a local checkout directory",
        )),
    }
}

fn parse_update_mode(raw: Option<&str>) -> Result<CorpusMode, CorpusFailure> {
    match raw {
        None | Some("0" | "no") => Ok(CorpusMode::Verify),
        Some("1") => Ok(CorpusMode::Update),
        Some(other) => Err(CorpusFailure::new(
            FailureKind::InvalidEnv,
            format!("invalid CRAPTS_CORPUS_UPDATE value \"{other}\" (expected unset, 0, no, or 1)"),
        )),
    }
}

fn parse_pin(raw: &str) -> Result<String, CorpusFailure> {
    let line = raw
        .strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw);
    let valid_sha = line.len() == 40
        && line
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if valid_sha {
        Ok(line.to_owned())
    } else {
        Err(CorpusFailure::new(
            FailureKind::InvalidPin,
            format!("invalid corpus pin {raw:?}: expected a single 40-character lowercase hex SHA"),
        ))
    }
}

fn git_output(
    corpus_dir: &Path,
    args: &[&str],
) -> Result<String, CorpusFailure> {
    let output = Command::new("git")
        .arg("-C")
        .arg(corpus_dir)
        .args(args)
        .output()
        .map_err(|e| {
            CorpusFailure::new(
                FailureKind::GitCheck,
                format!(
                    "failed to run git {args:?} in {}: {e}",
                    corpus_dir.display()
                ),
            )
        })?;
    if !output.status.success() {
        return Err(CorpusFailure::new(
            FailureKind::GitCheck,
            format!(
                "git {args:?} failed in {}: {}",
                corpus_dir.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| {
        CorpusFailure::new(
            FailureKind::GitCheck,
            format!("non-UTF-8 output from git {args:?}: {e}"),
        )
    })
}

fn verify_checkout(
    corpus_dir: &Path,
    expected_pin: &str,
) -> Result<(), CorpusFailure> {
    if !corpus_dir.is_dir() {
        return Err(CorpusFailure::new(
            FailureKind::MissingCorpusDir,
            format!("corpus directory does not exist: {}", corpus_dir.display()),
        ));
    }

    let head = git_output(corpus_dir, &["rev-parse", "HEAD"])?;
    let actual_pin = head.trim();
    if actual_pin != expected_pin {
        return Err(CorpusFailure::new(
            FailureKind::PinMismatch,
            format!(
                "corpus checkout HEAD mismatch in {}: expected {expected_pin}, actual {actual_pin}",
                corpus_dir.display()
            ),
        ));
    }

    let status = git_output(
        corpus_dir,
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    if !status.is_empty() {
        let dirty: Vec<&str> = status.lines().take(10).collect();
        return Err(CorpusFailure::new(
            FailureKind::DirtyWorktree,
            format!(
                "corpus checkout is dirty in {}:\n{}",
                corpus_dir.display(),
                dirty.join("\n")
            ),
        ));
    }

    Ok(())
}

fn decompress_coverage(gz_path: &Path) -> Result<NamedTempFile, CorpusFailure> {
    let compressed = fs::File::open(gz_path).map_err(|e| {
        CorpusFailure::new(
            FailureKind::Decompression,
            format!("failed to open {}: {e}", gz_path.display()),
        )
    })?;
    let mut decoder = GzDecoder::new(compressed);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).map_err(|e| {
        CorpusFailure::new(
            FailureKind::Decompression,
            format!("failed to decompress {}: {e}", gz_path.display()),
        )
    })?;

    let mut temp = tempfile::Builder::new()
        .prefix("crapts-corpus-coverage-")
        .suffix(".json")
        .tempfile()
        .map_err(|e| {
            CorpusFailure::new(
                FailureKind::Decompression,
                format!("failed to create temporary coverage file: {e}"),
            )
        })?;
    temp.write_all(&bytes).map_err(|e| {
        CorpusFailure::new(
            FailureKind::Decompression,
            format!("failed to write temporary coverage file: {e}"),
        )
    })?;
    temp.flush().map_err(|e| {
        CorpusFailure::new(
            FailureKind::Decompression,
            format!("failed to flush temporary coverage file: {e}"),
        )
    })?;
    Ok(temp)
}

fn path_variants(path: &Path) -> Vec<String> {
    let mut candidates = vec![path.to_path_buf()];
    if let Ok(canonical) = path.canonicalize() {
        candidates.push(canonical);
    }

    let mut needles = BTreeSet::new();
    for candidate in candidates {
        let native = candidate.to_string_lossy();
        let trimmed = native.trim_end_matches(['/', '\\']);
        if !trimmed.is_empty() {
            needles.insert(trimmed.to_owned());
            let slashed = trimmed.replace('\\', "/");
            if !slashed.is_empty() {
                needles.insert(slashed);
            }
            if trimmed.contains('\\') {
                needles.insert(trimmed.replace('\\', "\\\\"));
            }
        }
    }
    needles.into_iter().collect()
}

fn check_path_leaks(
    stdout: &str,
    stderr: &str,
    scope: &LeakScope<'_>,
) -> Result<(), CorpusFailure> {
    let mut checks: Vec<(&str, Vec<String>)> = vec![
        ("checkout path", path_variants(scope.corpus_dir)),
        (
            "temp coverage path",
            path_variants(scope.temp_coverage_path),
        ),
    ];
    if let Some(home_dir) = scope.home {
        let home_needles = path_variants(home_dir);
        if !home_needles.is_empty() {
            checks.push(("$HOME", home_needles));
        }
    }

    let mut leaks = Vec::new();
    for (stream, text) in [("stdout", stdout), ("stderr", stderr)] {
        for (kind, needles) in &checks {
            if let Some(hit) = needles.iter().find(|needle| text.contains(needle.as_str())) {
                leaks.push(format!("{stream} leaked {kind} ({hit})"));
            }
        }
    }

    if leaks.is_empty() {
        Ok(())
    } else {
        Err(CorpusFailure::new(
            FailureKind::PathLeak,
            format!("path leak detected: {}", leaks.join("; ")),
        ))
    }
}

fn validate_and_normalize(raw_stdout: &str) -> Result<Value, CorpusFailure> {
    let mut value: Value = serde_json::from_str(raw_stdout).map_err(|e| {
        CorpusFailure::new(
            FailureKind::InvalidOutput,
            format!("crapts stdout is not valid JSON: {e}"),
        )
    })?;

    let errors: Vec<String> = report_v1_validator()
        .iter_errors(&value)
        .map(|err| err.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(CorpusFailure::new(
            FailureKind::InvalidOutput,
            format!(
                "crapts stdout failed schemas/crapts/report-v1.json validation:\n{}",
                errors.join("\n")
            ),
        ));
    }

    let pkg_version = env!("CARGO_PKG_VERSION");
    let actual_version = value.get("version").and_then(Value::as_str);
    if actual_version != Some(pkg_version) {
        return Err(CorpusFailure::new(
            FailureKind::InvalidOutput,
            format!("report version mismatch: expected {pkg_version:?}, actual {actual_version:?}"),
        ));
    }

    let map = value.as_object_mut().ok_or_else(|| {
        CorpusFailure::new(
            FailureKind::InvalidOutput,
            "report top-level JSON must be an object",
        )
    })?;
    map.insert(
        "version".to_owned(),
        Value::String(VERSION_PLACEHOLDER.to_owned()),
    );
    Ok(value)
}

fn row_key(row: &Value) -> Option<String> {
    let file = row.get("file")?.as_str()?;
    let function = row.get("function")?.as_str()?;
    Some(format!("{file}::{function}"))
}

fn collect_duplicate_keys(
    side: &str,
    doc: &Value,
) -> Vec<String> {
    let Some(entries) = doc.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in entries {
        if let Some(key) = row_key(row) {
            let line = row
                .get("line")
                .map_or_else(|| "<missing>".to_owned(), ToString::to_string);
            seen.entry(key).or_default().push(line);
        }
    }
    seen.into_iter()
        .filter(|(_, lines)| lines.len() > 1)
        .map(|(key, lines)| format!("{side}: duplicate key {key} at lines {}", lines.join(", ")))
        .collect()
}

fn index_rows(doc: &Value) -> BTreeMap<String, &Value> {
    let mut map = BTreeMap::new();
    if let Some(entries) = doc.get("entries").and_then(Value::as_array) {
        for row in entries {
            if let Some(key) = row_key(row) {
                map.insert(key, row);
            }
        }
    }
    map
}

fn format_opt_value(val: Option<&Value>) -> String {
    val.map_or_else(|| "<missing>".to_owned(), Value::to_string)
}

fn diff_objects(
    expected: &Map<String, Value>,
    actual: &Map<String, Value>,
    skip_key: Option<&str>,
) -> Vec<(String, String, String)> {
    let keys: BTreeSet<&str> = expected
        .keys()
        .chain(actual.keys())
        .map(String::as_str)
        .filter(|key| Some(*key) != skip_key)
        .collect();
    let mut diffs = Vec::new();
    for key in keys {
        let exp = expected.get(key);
        let act = actual.get(key);
        if exp != act {
            diffs.push((key.to_owned(), format_opt_value(exp), format_opt_value(act)));
        }
    }
    diffs
}

fn diff_stderr(
    expected: &str,
    actual: &str,
) -> Vec<String> {
    if expected == actual {
        return Vec::new();
    }
    let exp_lines: Vec<&str> = expected.split('\n').collect();
    let act_lines: Vec<&str> = actual.split('\n').collect();
    let max_len = exp_lines.len().max(act_lines.len());
    let mut diffs = Vec::new();
    for idx in 0..max_len {
        let exp = exp_lines.get(idx).copied();
        let act = act_lines.get(idx).copied();
        if exp != act {
            diffs.push(format!(
                "stderr line {}: expected {:?}, actual {:?}",
                idx + 1,
                exp.unwrap_or("<eof>"),
                act.unwrap_or("<eof>")
            ));
        }
    }
    diffs
}

fn diff_rows(
    expected: &Value,
    actual: &Value,
    sections: &mut Vec<String>,
) -> (Vec<String>, Vec<String>) {
    let exp_rows = index_rows(expected);
    let act_rows = index_rows(actual);
    let mut added_rows = Vec::new();
    let mut removed_rows = Vec::new();

    for key in act_rows.keys() {
        if !exp_rows.contains_key(key) {
            added_rows.push(key.clone());
            sections.push(format!("added row {key}"));
        }
    }
    for key in exp_rows.keys() {
        if !act_rows.contains_key(key) {
            removed_rows.push(key.clone());
            sections.push(format!("removed row {key}"));
        }
    }
    for (key, exp_row) in &exp_rows {
        if let Some(act_row) = act_rows.get(key)
            && exp_row != act_row
        {
            if let (Some(exp_map), Some(act_map)) = (exp_row.as_object(), act_row.as_object()) {
                for (field, exp_val, act_val) in diff_objects(exp_map, act_map, None) {
                    sections.push(format!(
                        "changed row {key} field {field}: expected {exp_val}, actual {act_val}"
                    ));
                }
            } else {
                sections.push(format!(
                    "changed row {key}: expected {exp_row}, actual {act_row}"
                ));
            }
        }
    }
    (added_rows, removed_rows)
}

fn compare_reports(
    expected: &Value,
    actual: &Value,
    expected_stderr: &str,
    actual_stderr: &str,
) -> Result<(), CorpusFailure> {
    if expected == actual && expected_stderr == actual_stderr {
        return Ok(());
    }

    let mut duplicates = collect_duplicate_keys("golden", expected);
    duplicates.extend(collect_duplicate_keys("actual", actual));
    if !duplicates.is_empty() {
        return Err(CorpusFailure::new(
            FailureKind::Drift,
            duplicates.join("\n"),
        ));
    }

    let mut sections = Vec::new();
    if let (Some(exp_obj), Some(act_obj)) = (expected.as_object(), actual.as_object()) {
        for (member, exp_val, act_val) in diff_objects(exp_obj, act_obj, Some("entries")) {
            sections.push(format!(
                "outer member {member}: expected {exp_val}, actual {act_val}"
            ));
        }
    } else if expected != actual {
        sections.push(format!("document: expected {expected}, actual {actual}"));
    }

    let (added_rows, removed_rows) = diff_rows(expected, actual, &mut sections);

    if expected.get("entries") != actual.get("entries")
        && sections.is_empty()
        && let (Some(exp_arr), Some(act_arr)) = (
            expected.get("entries").and_then(Value::as_array),
            actual.get("entries").and_then(Value::as_array),
        )
    {
        let diff_idx = exp_arr
            .iter()
            .zip(act_arr.iter())
            .position(|(lhs, rhs)| lhs != rhs)
            .unwrap_or_else(|| exp_arr.len().min(act_arr.len()));
        sections.push(format!(
            "entries order differs at index {diff_idx}: expected {}, actual {}",
            format_opt_value(exp_arr.get(diff_idx)),
            format_opt_value(act_arr.get(diff_idx))
        ));
    }

    sections.extend(diff_stderr(expected_stderr, actual_stderr));
    if sections.is_empty() {
        sections.push(format!(
            "corpus output differs: expected {expected}, actual {actual}"
        ));
    }

    Err(CorpusFailure {
        kind: FailureKind::Drift,
        message: sections.join("\n"),
        added_rows,
        removed_rows,
    })
}

fn finalize_corpus_output(
    raw_stdout: &str,
    raw_stderr: &str,
    scope: &LeakScope<'_>,
    fixture_dir: &Path,
    mode: CorpusMode,
) -> Result<(), CorpusFailure> {
    check_path_leaks(raw_stdout, raw_stderr, scope)?;
    let normalized = validate_and_normalize(raw_stdout)?;

    let golden_json_path = fixture_dir.join("golden.json");
    let golden_stderr_path = fixture_dir.join("golden.stderr");

    match mode {
        CorpusMode::Verify => {
            let golden_raw = fs::read_to_string(&golden_json_path).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to read {}: {e}", golden_json_path.display()),
                )
            })?;
            let golden_value: Value = serde_json::from_str(&golden_raw).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to parse {}: {e}", golden_json_path.display()),
                )
            })?;
            let golden_stderr = fs::read_to_string(&golden_stderr_path).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to read {}: {e}", golden_stderr_path.display()),
                )
            })?;
            compare_reports(&golden_value, &normalized, &golden_stderr, raw_stderr)
        },
        CorpusMode::Update => {
            let mut pretty = serde_json::to_string_pretty(&normalized).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to serialize normalized golden JSON: {e}"),
                )
            })?;
            pretty.push('\n');
            fs::write(&golden_json_path, pretty).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to write {}: {e}", golden_json_path.display()),
                )
            })?;
            fs::write(&golden_stderr_path, raw_stderr).map_err(|e| {
                CorpusFailure::new(
                    FailureKind::Io,
                    format!("failed to write {}: {e}", golden_stderr_path.display()),
                )
            })?;
            Ok(())
        },
    }
}

fn log_elapsed_duration(
    elapsed: Duration,
    step_summary_path: Option<&Path>,
) {
    let line = format!("crapts corpus elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("{line}");
    if let Some(summary_path) = step_summary_path
        && let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(summary_path)
    {
        let _ = writeln!(file, "{line}");
    }
}

fn run_corpus(
    corpus_dir: &Path,
    fixture_dir: &Path,
    expected_pin: &str,
    mode: CorpusMode,
) -> Result<(), CorpusFailure> {
    run_corpus_with_summary(corpus_dir, fixture_dir, expected_pin, mode, None)
}

fn run_corpus_with_summary(
    corpus_dir: &Path,
    fixture_dir: &Path,
    expected_pin: &str,
    mode: CorpusMode,
    step_summary_path: Option<&Path>,
) -> Result<(), CorpusFailure> {
    verify_checkout(corpus_dir, expected_pin)?;
    let temp_coverage = decompress_coverage(&fixture_dir.join("coverage-final.json.gz"))?;

    let mut command = Command::new(env!("CARGO_BIN_EXE_crapts"));
    command
        .current_dir(corpus_dir)
        .arg(".")
        .arg("--coverage")
        .arg(temp_coverage.path())
        .args(["--format", "json", "--sort", "file"])
        .env("NO_COLOR", "1");
    for key in [
        "FORCE_COLOR",
        "GITHUB_SERVER_URL",
        "GITHUB_REPOSITORY",
        "GITHUB_SHA",
        "CRAPTS_CORPUS_UPDATE",
    ] {
        command.env_remove(key);
    }

    let started = Instant::now();
    let output = command.output().map_err(|e| {
        CorpusFailure::new(
            FailureKind::Command,
            format!("failed to execute crapts in {}: {e}", corpus_dir.display()),
        )
    })?;
    log_elapsed_duration(started.elapsed(), step_summary_path);

    let raw_stdout = String::from_utf8(output.stdout).map_err(|e| {
        CorpusFailure::new(
            FailureKind::InvalidOutput,
            format!("crapts stdout is not valid UTF-8: {e}"),
        )
    })?;
    let raw_stderr = String::from_utf8(output.stderr).map_err(|e| {
        CorpusFailure::new(
            FailureKind::InvalidOutput,
            format!("crapts stderr is not valid UTF-8: {e}"),
        )
    })?;

    if !output.status.success() {
        return Err(CorpusFailure::new(
            FailureKind::Command,
            format!(
                "crapts exited with {:?}\nstderr:\n{raw_stderr}",
                output.status.code()
            ),
        ));
    }

    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let scope = LeakScope {
        corpus_dir,
        temp_coverage_path: temp_coverage.path(),
        home: home.as_deref(),
    };
    finalize_corpus_output(&raw_stdout, &raw_stderr, &scope, fixture_dir, mode)
}

fn write_gz_json(
    path: &Path,
    value: &Value,
) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&serde_json::to_vec(value).expect("serialize Istanbul fixture"))
        .expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");
    fs::write(path, compressed).expect("write coverage-final.json.gz");
}

fn git_run(
    dir: &Path,
    args: &[&str],
) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Corpus Test")
        .env("GIT_AUTHOR_EMAIL", "corpus@example.test")
        .env("GIT_COMMITTER_NAME", "Corpus Test")
        .env("GIT_COMMITTER_EMAIL", "corpus@example.test")
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 git output")
}

struct TempCorpus {
    repo_dir: tempfile::TempDir,
    fixture_dir: tempfile::TempDir,
}

impl TempCorpus {
    fn new(
        files: &[(&str, &str)],
        coverage: &Value,
    ) -> (Self, String) {
        let repo_dir = tempfile::tempdir().expect("temp repo");
        let fixture_dir = tempfile::tempdir().expect("temp fixture");
        git_run(repo_dir.path(), &["init"]);
        for (rel, content) in files {
            let full = repo_dir.path().join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(&full, content).expect("write source");
        }
        git_run(repo_dir.path(), &["add", "."]);
        git_run(repo_dir.path(), &["commit", "-m", "initial"]);
        let head = git_run(repo_dir.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        write_gz_json(&fixture_dir.path().join("coverage-final.json.gz"), coverage);
        (
            Self {
                repo_dir,
                fixture_dir,
            },
            head,
        )
    }

    fn commit_file(
        &self,
        rel: &str,
        content: &str,
    ) -> String {
        let full = self.repo_dir.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&full, content).expect("write source");
        git_run(self.repo_dir.path(), &["add", "."]);
        git_run(self.repo_dir.path(), &["commit", "-m", "update"]);
        git_run(self.repo_dir.path(), &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    }

    fn record_baseline(
        &self,
        pin: &str,
    ) {
        run_corpus(
            self.repo_dir.path(),
            self.fixture_dir.path(),
            pin,
            CorpusMode::Update,
        )
        .expect("record temp baseline");
    }

    fn verify_err(
        &self,
        pin: &str,
    ) -> CorpusFailure {
        run_corpus(
            self.repo_dir.path(),
            self.fixture_dir.path(),
            pin,
            CorpusMode::Verify,
        )
        .expect_err("expected verify failure")
    }

    fn golden_json(&self) -> Value {
        serde_json::from_str(
            &fs::read_to_string(self.fixture_dir.path().join("golden.json"))
                .expect("read golden.json"),
        )
        .expect("parse golden.json")
    }

    fn write_golden_json(
        &self,
        doc: &Value,
    ) {
        let mut pretty = serde_json::to_string_pretty(doc).expect("serialize golden");
        pretty.push('\n');
        fs::write(self.fixture_dir.path().join("golden.json"), pretty).expect("write golden.json");
    }
}

fn find_row_mut<'a>(
    doc: &'a mut Value,
    file: &str,
    function: &str,
) -> &'a mut Value {
    doc["entries"]
        .as_array_mut()
        .expect("entries array")
        .iter_mut()
        .find(|row| row["file"] == file && row["function"] == function)
        .unwrap_or_else(|| panic!("row {file}::{function} not found"))
}

fn istanbul_for_a(
    kept_hit: u64,
    extra_hit: u64,
) -> Value {
    json!({
        "src/a.ts": {
            "path": "src/a.ts",
            "statementMap": {
                "0": { "start": { "line": 2, "column": 2 }, "end": { "line": 2, "column": 11 } },
                "1": { "start": { "line": 3, "column": 2 }, "end": { "line": 3, "column": 11 } },
                "2": { "start": { "line": 7, "column": 2 }, "end": { "line": 7, "column": 11 } }
            },
            "s": {
                "0": kept_hit,
                "1": extra_hit,
                "2": 1
            }
        }
    })
}

fn base_source_a() -> &'static str {
    "export function kept(x: boolean) {\n  const y = x ? 1 : 2;\n  return y;\n}\n\nexport function gone() {\n  return 1;\n}\n"
}

fn assert_source_and_coverage_drift(
    corpus: &TempCorpus,
    coverage: &Value,
) -> String {
    let added_src = format!(
        "{}\nexport function added() {{\n  return 2;\n}}\n",
        base_source_a()
    );
    let added_pin = corpus.commit_file("src/a.ts", &added_src);
    let err = corpus.verify_err(&added_pin);
    assert_eq!(err.kind, FailureKind::Drift);
    assert!(
        err.to_string().contains("added row src/a.ts::added"),
        "{err}"
    );

    let removed_src =
        "export function kept(x: boolean) {\n  const y = x ? 1 : 2;\n  return y;\n}\n";
    let removed_pin = corpus.commit_file("src/a.ts", removed_src);
    let err = corpus.verify_err(&removed_pin);
    assert!(
        err.to_string().contains("removed row src/a.ts::gone"),
        "{err}"
    );

    let shifted_src = format!("\n{}", base_source_a());
    let shifted_pin = corpus.commit_file("src/a.ts", &shifted_src);
    let msg = corpus.verify_err(&shifted_pin).to_string();
    assert!(
        msg.contains("src/a.ts::kept")
            && msg.contains("line")
            && msg.contains("expected 1")
            && msg.contains("actual 2"),
        "{msg}"
    );

    let branched_src = "export function kept(x: boolean) {\n  const y = x ? (x ? 1 : 3) : 2;\n  return y;\n}\n\nexport function gone() {\n  return 1;\n}\n";
    let branched_pin = corpus.commit_file("src/a.ts", branched_src);
    let msg = corpus.verify_err(&branched_pin).to_string();
    assert!(
        msg.contains("src/a.ts::kept")
            && msg.contains("cyclomatic")
            && msg.contains("expected 2")
            && msg.contains("actual 3"),
        "{msg}"
    );

    let restored_pin = corpus.commit_file("src/a.ts", base_source_a());
    write_gz_json(
        &corpus.fixture_dir.path().join("coverage-final.json.gz"),
        &istanbul_for_a(1, 1),
    );
    let msg = corpus.verify_err(&restored_pin).to_string();
    assert!(
        msg.contains("changed row src/a.ts::kept field coverage: expected 50.0, actual 100.0"),
        "{msg}"
    );
    write_gz_json(
        &corpus.fixture_dir.path().join("coverage-final.json.gz"),
        coverage,
    );
    restored_pin
}

fn assert_row_and_outer_golden_drift(
    corpus: &TempCorpus,
    baseline_golden: &Value,
    restored_pin: &str,
) {
    let mut mutated = baseline_golden.clone();
    find_row_mut(&mut mutated, "src/a.ts", "kept")["crap"] = json!(99.5);
    corpus.write_golden_json(&mutated);
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("src/a.ts::kept") && msg.contains("crap") && msg.contains("99.5"),
        "{msg}"
    );

    let mut mutated = baseline_golden.clone();
    find_row_mut(&mut mutated, "src/a.ts", "kept")["uncovered"] = json!([{"start": 2, "end": 4}]);
    corpus.write_golden_json(&mutated);
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("src/a.ts::kept")
            && msg.contains("uncovered")
            && msg.contains("\"start\":2")
            && msg.contains("\"end\":4")
            && msg.contains("\"start\":3")
            && msg.contains("\"end\":3"),
        "{msg}"
    );

    let mut mutated_fp = baseline_golden.clone();
    find_row_mut(&mut mutated_fp, "src/a.ts", "kept")["fingerprint"] = json!("deadbeef");
    corpus.write_golden_json(&mutated_fp);
    let msg_v = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg_v.contains("src/a.ts::kept")
            && msg_v.contains("fingerprint")
            && msg_v.contains("deadbeef"),
        "{msg_v}"
    );
    let mut without_fp = baseline_golden.clone();
    find_row_mut(&mut without_fp, "src/a.ts", "kept")
        .as_object_mut()
        .unwrap()
        .remove("fingerprint");
    corpus.write_golden_json(&without_fp);
    let msg_g = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg_g.contains("src/a.ts::kept")
            && msg_g.contains("fingerprint")
            && msg_g.contains("<missing>"),
        "{msg_g}"
    );
    let msg_a = compare_reports(baseline_golden, &without_fp, "", "")
        .unwrap_err()
        .to_string();
    assert!(
        msg_a.contains("src/a.ts::kept")
            && msg_a.contains("fingerprint")
            && msg_a.contains("<missing>"),
        "{msg_a}"
    );

    let mut mutated = baseline_golden.clone();
    mutated["summary"]["total"] = json!(99);
    corpus.write_golden_json(&mutated);
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("summary") && msg.contains("99") && msg.contains("\"total\":2"),
        "{msg}"
    );

    let mut mutated = baseline_golden.clone();
    mutated["diagnostics"]["coverage_files"] = json!(42);
    corpus.write_golden_json(&mutated);
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("diagnostics")
            && msg.contains("\"coverage_files\":42")
            && msg.contains("\"coverage_files\":1"),
        "{msg}"
    );

    let mut mutated = baseline_golden.clone();
    mutated["notices"] = json!(["stale notice"]);
    corpus.write_golden_json(&mutated);
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("notices") && msg.contains("stale notice") && msg.contains("[]"),
        "{msg}"
    );

    for (field, replacement) in [
        ("coverage_kind", json!("lines")),
        ("threshold", json!(15.0)),
        ("extra_member", json!(true)),
    ] {
        let mut mutated = baseline_golden.clone();
        mutated[field] = replacement.clone();
        corpus.write_golden_json(&mutated);
        let msg = corpus.verify_err(restored_pin).to_string();
        assert!(
            msg.contains(field) && msg.contains(&replacement.to_string()),
            "{field}: {msg}"
        );
    }
}

fn assert_stderr_order_and_duplicate_drift(
    corpus: &TempCorpus,
    baseline_golden: &Value,
    restored_pin: &str,
) {
    corpus.write_golden_json(baseline_golden);
    fs::write(
        corpus.fixture_dir.path().join("golden.stderr"),
        "expected warning line\n",
    )
    .expect("write golden.stderr");
    let msg = corpus.verify_err(restored_pin).to_string();
    assert!(
        msg.contains("stderr") && msg.contains("expected warning line"),
        "{msg}"
    );

    let mut swapped = baseline_golden.clone();
    swapped["entries"].as_array_mut().unwrap().reverse();
    let order_msg = compare_reports(baseline_golden, &swapped, "", "")
        .unwrap_err()
        .to_string();
    assert!(order_msg.contains("entries order"), "{order_msg}");

    let with_dup = json!({
        "entries": [
            {"file": "src/a.ts", "function": "dup", "line": 1, "cyclomatic": 1, "coverage": 100.0, "crap": 1.0},
            {"file": "src/a.ts", "function": "dup", "line": 5, "cyclomatic": 1, "coverage": 100.0, "crap": 1.0}
        ]
    });
    for err in [
        compare_reports(&with_dup, baseline_golden, "", "").unwrap_err(),
        compare_reports(baseline_golden, &with_dup, "", "").unwrap_err(),
    ] {
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate key src/a.ts::dup at lines 1, 5"),
            "{msg}"
        );
    }
}

fn empty_valid_report(version: &str) -> Value {
    json!({
        "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
        "version": version,
        "name_scheme": 1,
        "coverage_kind": "statements",
        "threshold": 30.0,
        "selection": null,
        "notices": [],
        "summary": {
            "total": 0,
            "crappy": 0,
            "regressed": null,
            "improved": null,
            "new": null,
            "moved": null,
            "unchanged": null,
            "removed": null,
            "changed_failures": null
        },
        "entries": [],
        "diagnostics": {
            "analyzed_files": 0,
            "coverage_files": 0,
            "matched_files": 0,
            "source_only": {"count": 0, "examples": []},
            "coverage_only": {"count": 0, "examples": []}
        }
    })
}

fn assert_path_leak_failures(
    fixture_dir: &Path,
    sentinel_json: &[u8],
    sentinel_stderr: &[u8],
) {
    let link_base = tempfile::tempdir().unwrap();
    let real_checkout = link_base.path().join("real-checkout");
    fs::create_dir(&real_checkout).unwrap();
    #[cfg(unix)]
    let checkout_arg = {
        let link = link_base.path().join("link-checkout");
        std::os::unix::fs::symlink(&real_checkout, &link).unwrap();
        link
    };
    #[cfg(not(unix))]
    let checkout_arg = real_checkout.clone();

    let canonical_checkout = real_checkout.canonicalize().unwrap();
    let temp_cov = link_base.path().join("temp-cov-1234.json");
    let home = PathBuf::from("/fake-corpus-home");
    let scope = LeakScope {
        corpus_dir: &checkout_arg,
        temp_coverage_path: &temp_cov,
        home: Some(&home),
    };

    let valid_json = empty_valid_report(env!("CARGO_PKG_VERSION"));
    let clean_stdout = serde_json::to_string(&valid_json).unwrap();

    for (leaked_text, expected_kind) in [
        (
            format!("{}/src/a.ts", checkout_arg.display()),
            "checkout path",
        ),
        (
            format!("{}/src/a.ts", canonical_checkout.display()),
            "checkout path",
        ),
        (temp_cov.display().to_string(), "temp coverage path"),
        (format!("{}/.config/crapts", home.display()), "$HOME"),
    ] {
        let mut leaky_json = valid_json.clone();
        leaky_json["notices"] = json!([leaked_text]);
        let leaky_stdout = serde_json::to_string(&leaky_json).unwrap();
        let leaky_stderr = format!("warning: {leaked_text}\n");
        for mode in [CorpusMode::Verify, CorpusMode::Update] {
            for (stdout, stderr, label) in [
                (leaky_stdout.as_str(), "", "stdout"),
                (clean_stdout.as_str(), leaky_stderr.as_str(), "stderr"),
            ] {
                let err =
                    finalize_corpus_output(stdout, stderr, &scope, fixture_dir, mode).unwrap_err();
                assert_eq!(err.kind, FailureKind::PathLeak);
                assert!(
                    err.to_string().contains(expected_kind),
                    "{label} leak ({expected_kind}): {err}"
                );
                assert_eq!(
                    fs::read(fixture_dir.join("golden.json")).unwrap(),
                    sentinel_json
                );
                assert_eq!(
                    fs::read(fixture_dir.join("golden.stderr")).unwrap(),
                    sentinel_stderr
                );
            }
        }
    }
}

#[test]
fn offline_packaging_and_windows_line_ending_invariants_hold() {
    let root = workspace_root();
    let attrs = fs::read_to_string(root.join(".gitattributes")).expect("read .gitattributes");
    assert!(
        attrs
            .lines()
            .any(|line| line.trim() == "crates/crapts/tests/fixtures/corpus/** -text"),
        ".gitattributes missing corpus -text rule:\n{attrs}"
    );
    assert!(
        attrs.lines().any(|line| line.trim() == "*.gz binary"),
        ".gitattributes missing *.gz binary rule:\n{attrs}"
    );

    let pin_path = root.join("crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt");
    let raw_pin = fs::read_to_string(&pin_path).expect("read pin.txt");
    assert_eq!(raw_pin, format!("{PINNED_SHA}\n"));
    assert_eq!(parse_pin(&raw_pin).unwrap(), PINNED_SHA);
    assert_eq!(parse_pin(&format!("{PINNED_SHA}\r\n")).unwrap(), PINNED_SHA);
    assert_eq!(parse_pin(PINNED_SHA).unwrap(), PINNED_SHA);

    for malformed in [
        format!("{PINNED_SHA}\n\n"),
        format!("{PINNED_SHA}\r\n\r\n"),
        format!("{PINNED_SHA}\nextra\n"),
        format!(" {PINNED_SHA}\n"),
        format!("{}\n", PINNED_SHA.to_ascii_uppercase()),
        "2c3dcc0\n".to_owned(),
        String::new(),
    ] {
        assert_eq!(
            parse_pin(&malformed).unwrap_err().kind,
            FailureKind::InvalidPin,
            "expected InvalidPin for {malformed:?}"
        );
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .args([
            "package",
            "--list",
            "-p",
            "crapts",
            "--allow-dirty",
            "--locked",
            "--offline",
        ])
        .current_dir(&root)
        .output()
        .expect("run cargo package --list");
    assert!(
        output.status.success(),
        "cargo package --list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8(output.stdout).expect("utf-8 package listing");
    let leaked: Vec<&str> = listing
        .lines()
        .filter(|line| {
            line.replace('\\', "/")
                .starts_with("tests/fixtures/corpus/")
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "corpus fixtures leaked into crapts package: {leaked:?}"
    );
}

fn is_checkout_relative(path: &str) -> bool {
    let has_drive =
        path.len() >= 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':';
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !has_drive
        && !path.split('/').any(|part| part == "..")
}

fn is_path_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '/' | '\\' | '.' | '_' | '-' | '~')
}

fn mentions_path(
    text: &str,
    path: &str,
) -> bool {
    text.match_indices(path).any(|(at, _)| {
        let head = &text[..at];
        let before = head.chars().next_back();
        let starts = !before.is_some_and(is_path_char) || head.ends_with("file://");
        let after = text[at + path.len()..].chars().next();
        starts && after.is_none_or(|ch| matches!(ch, '/' | '\\') || !is_path_char(ch))
    })
}

fn assert_no_leaked_prefixes(
    label: &str,
    text: &str,
) {
    let mut forbidden = vec![
        "/Users".to_owned(),
        "/home".to_owned(),
        "/private/var/folders".to_owned(),
        "/tmp".to_owned(),
    ];
    forbidden.extend(path_variants(&workspace_root()));
    forbidden.extend(path_variants(&std::env::temp_dir()));
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        forbidden.extend(path_variants(Path::new(&home)));
    }
    for needle in forbidden {
        assert!(
            !mentions_path(text, &needle),
            "{label} contains forbidden path prefix {needle:?}"
        );
    }
    for drive in ["C:\\", "C:/"] {
        let has_drive = text.match_indices(drive).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            !before.is_some_and(is_path_char)
        });
        assert!(
            !has_drive,
            "{label} contains Windows drive prefix {drive:?}"
        );
    }
}

fn assert_coverage_archive_relative(gz_path: &Path) {
    let decompressed = decompress_coverage(gz_path).expect("decompress coverage-final.json.gz");
    let istanbul_raw = fs::read_to_string(decompressed.path()).expect("read decompressed coverage");
    let istanbul: Value =
        serde_json::from_str(&istanbul_raw).expect("parse decompressed Istanbul JSON");
    let map = istanbul
        .as_object()
        .expect("Istanbul coverage must be a JSON object");
    assert!(!map.is_empty(), "Istanbul coverage map must be non-empty");
    for (key, entry) in map {
        assert!(
            is_checkout_relative(key),
            "coverage map key is not checkout-relative: {key:?}"
        );
        let inner_path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("coverage entry {key:?} missing string path"));
        assert!(
            is_checkout_relative(inner_path),
            "coverage inner path is not checkout-relative: {inner_path:?}"
        );
        assert_eq!(key, inner_path, "coverage map key differs from inner path");
    }
}

fn assert_coverage_meta(
    fixture_dir: &Path,
    gz_path: &Path,
    golden_json_path: &Path,
    golden_stderr_path: &Path,
) {
    let meta: Value = serde_json::from_str(
        &fs::read_to_string(fixture_dir.join("coverage-meta.json"))
            .expect("read coverage-meta.json"),
    )
    .expect("parse coverage-meta.json");
    assert_eq!(meta["commit"], PINNED_SHA);
    let command = meta["command"]
        .as_str()
        .expect("coverage-meta.json string command");
    for token in [
        "vitest run",
        "--coverage",
        "--coverage.reporter=json",
        "--coverage.provider=",
        "--coverage.reportOnFailure=true",
    ] {
        assert!(
            command.contains(token),
            "coverage-meta.json command missing {token:?}: {command}"
        );
    }
    assert!(
        meta["provider"]
            .as_str()
            .is_some_and(|provider| !provider.is_empty()),
        "coverage-meta.json missing provider"
    );
    assert_eq!(meta["reportOnFailure"], json!(true));
    for tool in ["node", "yarn", "vitest"] {
        assert!(
            meta["versions"][tool]
                .as_str()
                .is_some_and(|version| !version.is_empty()),
            "coverage-meta.json missing versions.{tool}"
        );
    }
    assert_eq!(
        meta["artifacts"]["coverage_final_json_gz_bytes"].as_u64(),
        Some(fs::metadata(gz_path).expect("stat gz").len())
    );
    assert_eq!(
        meta["artifacts"]["golden_json_bytes"].as_u64(),
        Some(
            fs::metadata(golden_json_path)
                .expect("stat golden.json")
                .len()
        )
    );
    assert_eq!(
        meta["artifacts"]["golden_stderr_bytes"].as_u64(),
        Some(
            fs::metadata(golden_stderr_path)
                .expect("stat golden.stderr")
                .len()
        )
    );
}

fn verify_offline_corpus_fixtures() {
    let fixture_dir = workspace_root().join("crates/crapts/tests/fixtures/corpus/excalidraw");
    let gz_path = fixture_dir.join("coverage-final.json.gz");
    let golden_json_path = fixture_dir.join("golden.json");
    let golden_stderr_path = fixture_dir.join("golden.stderr");

    assert_coverage_archive_relative(&gz_path);
    assert_coverage_meta(
        &fixture_dir,
        &gz_path,
        &golden_json_path,
        &golden_stderr_path,
    );

    let golden_json_raw = fs::read_to_string(&golden_json_path).expect("read golden.json");
    let golden_stderr_raw = fs::read_to_string(&golden_stderr_path).expect("read golden.stderr");
    assert_no_leaked_prefixes("golden.json", &golden_json_raw);
    assert_no_leaked_prefixes("golden.stderr", &golden_stderr_raw);
    assert!(!mentions_path("\"/tmpl/foo\"", "/tmp"));
    assert!(mentions_path("\"/tmp/foo\"", "/tmp"));

    assert!(golden_json_raw.ends_with('\n'));
    assert!(golden_json_raw.starts_with("{\n  \""));
    let golden: Value = serde_json::from_str(&golden_json_raw).expect("parse golden.json");
    assert_eq!(golden["version"], VERSION_PLACEHOLDER);
    assert_eq!(
        golden["$schema"],
        "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json"
    );

    let mut schema_probe = golden.clone();
    schema_probe["version"] = Value::String(env!("CARGO_PKG_VERSION").to_owned());
    let schema_errors: Vec<String> = report_v1_validator()
        .iter_errors(&schema_probe)
        .map(|err| err.to_string())
        .collect();
    assert!(
        schema_errors.is_empty(),
        "golden.json fails schemas/crapts/report-v1.json:\n{}",
        schema_errors.join("\n")
    );

    let entries = golden["entries"]
        .as_array()
        .expect("golden.json entries array");
    assert!(!entries.is_empty(), "golden.json entries must be non-empty");
    for row in entries {
        let file = row["file"].as_str().expect("row file");
        assert!(
            is_checkout_relative(file),
            "golden.json entry file is not checkout-relative: {file:?}"
        );
    }
    for bucket in ["source_only", "coverage_only"] {
        let examples = golden["diagnostics"][bucket]["examples"]
            .as_array()
            .expect("diagnostics examples array");
        for example in examples {
            let path = example.as_str().expect("example path string");
            assert!(
                is_checkout_relative(path),
                "golden.json diagnostics.{bucket} example is not checkout-relative: {path:?}"
            );
        }
    }
}

fn snapshot_tree_hashes(root: &Path) -> BTreeMap<String, [u8; 32]> {
    fn walk(
        base: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, [u8; 32]>,
    ) {
        for entry in fs::read_dir(dir).expect("read fixture dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .expect("under fixture root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&path).expect("read fixture file");
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                out.insert(rel, digest);
            }
        }
    }
    let mut hashes = BTreeMap::new();
    walk(root, root, &mut hashes);
    hashes
}

#[test]
fn offline_coverage_provenance_and_golden_hygiene_invariants_hold() {
    verify_offline_corpus_fixtures();
}

#[test]
fn offline_test_suite_skips_the_live_corpus_checkout_without_touches_to_fixtures() {
    let corpus_fixtures = workspace_root().join("crates/crapts/tests/fixtures/corpus");
    let before = snapshot_tree_hashes(&corpus_fixtures);

    let exe = std::env::current_exe().expect("current test binary");
    let output = Command::new(&exe)
        .args(["--list", "--ignored"])
        .output()
        .expect("list ignored tests");
    assert!(output.status.success());
    let ignored = String::from_utf8(output.stdout).expect("utf-8 test list");
    assert!(
        ignored.lines().any(|line| {
            line.starts_with("pinned_excalidraw_corpus_matches_the_committed_golden:")
        }),
        "Excalidraw corpus test must be #[ignore]:\n{ignored}"
    );

    verify_offline_corpus_fixtures();

    let summary_file = NamedTempFile::new().expect("temp step summary");
    log_elapsed_duration(Duration::from_millis(1250), Some(summary_file.path()));
    assert_eq!(
        fs::read_to_string(summary_file.path()).expect("read step summary"),
        "crapts corpus elapsed: 1.25s\n"
    );

    let after = snapshot_tree_hashes(&corpus_fixtures);
    assert_eq!(
        before, after,
        "offline corpus checks created or modified fixture files"
    );
}

#[test]
#[ignore = "requires a local Excalidraw checkout specified by CRAPTS_CORPUS_DIR"]
fn pinned_excalidraw_corpus_matches_the_committed_golden() {
    let raw_dir = std::env::var("CRAPTS_CORPUS_DIR").ok();
    let corpus_dir = parse_corpus_dir(raw_dir.as_deref()).unwrap_or_else(|err| panic!("{err}"));
    let raw_update = std::env::var("CRAPTS_CORPUS_UPDATE").ok();
    let mode = parse_update_mode(raw_update.as_deref()).unwrap_or_else(|err| panic!("{err}"));
    let fixture_dir = workspace_root().join("crates/crapts/tests/fixtures/corpus/excalidraw");
    let raw_pin = fs::read_to_string(fixture_dir.join("pin.txt"))
        .unwrap_or_else(|e| panic!("read pin.txt: {e}"));
    let expected_pin = parse_pin(&raw_pin).unwrap_or_else(|err| panic!("{err}"));
    let step_summary = std::env::var_os("GITHUB_STEP_SUMMARY")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Err(err) = run_corpus_with_summary(
        &corpus_dir,
        &fixture_dir,
        &expected_pin,
        mode,
        step_summary.as_deref(),
    ) {
        panic!("{err}");
    }
}

fn yaml_block<'a>(
    lines: &[&'a str],
    header: &str,
) -> Vec<&'a str> {
    let depth = header.len() - header.trim_start().len();
    let mut rest = lines.iter().copied().skip_while(|line| *line != header);
    assert!(rest.next().is_some(), "missing YAML header {header:?}");
    rest.take_while(|line| line.trim().is_empty() || (line.len() - line.trim_start().len()) > depth)
        .collect()
}

fn filter_globs(
    text: &str,
    header: &str,
) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    yaml_block(&lines, header)
        .iter()
        .filter_map(|line| line.trim().strip_prefix("- '")?.strip_suffix('\''))
        .map(str::to_owned)
        .collect()
}

fn any_glob_matches(
    globs: &[String],
    path: &str,
) -> bool {
    globs.iter().any(|glob| {
        globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .unwrap_or_else(|e| panic!("glob {glob:?}: {e}"))
            .compile_matcher()
            .is_match(path)
    })
}

fn contains_40_hex_sha(text: &str) -> bool {
    let mut run = 0usize;
    for byte in text.bytes() {
        if byte.is_ascii_hexdigit() {
            run += 1;
            if run >= 40 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn assert_corpus_job_steps(corpus_job: &[&str]) {
    assert!(
        corpus_job.iter().any(|l| l.trim() == "needs: [changes]"),
        "corpus job missing needs: [changes]"
    );
    assert!(
        corpus_job.iter().any(|l| {
            l.trim()
                == "if: needs.changes.outputs.corpus == 'true' || github.event_name == 'workflow_dispatch'"
        }),
        "corpus job missing changes/workflow_dispatch if condition"
    );
    assert!(
        corpus_job
            .iter()
            .any(|l| l.trim() == "runs-on: ubuntu-latest"),
        "corpus job missing runs-on: ubuntu-latest"
    );
    assert!(
        corpus_job.iter().any(|l| l.trim() == "timeout-minutes: 20"),
        "corpus job missing timeout-minutes: 20"
    );
    assert!(
        corpus_job
            .iter()
            .any(|l| l.trim().contains("uses: dtolnay/rust-toolchain")),
        "corpus job missing dtolnay/rust-toolchain"
    );

    let runs: Vec<&str> = corpus_job
        .iter()
        .filter_map(|l| l.trim().strip_prefix("run: "))
        .collect();
    assert!(
        runs.contains(&"shellcheck -x scripts/corpus/*.sh"),
        "corpus job missing shellcheck step: {runs:?}"
    );
    assert!(
        runs.iter()
            .any(|cmd| cmd.contains("scripts/corpus/checkout.sh \"$RUNNER_TEMP/excalidraw\"")),
        "corpus job missing checkout.sh \"$RUNNER_TEMP/excalidraw\": {runs:?}"
    );
    assert!(
        runs.contains(&"cargo test --locked -p crapts --test corpus -- --ignored"),
        "corpus job missing locked ignored corpus test: {runs:?}"
    );
    assert_eq!(
        runs.last().copied(),
        Some("git diff --exit-code -- crates/crapts/tests/fixtures/corpus"),
        "corpus job must end with git diff --exit-code backstop: {runs:?}"
    );
    assert!(
        corpus_job
            .iter()
            .any(|l| l.trim() == "CRAPTS_CORPUS_DIR: ${{ runner.temp }}/excalidraw"),
        "corpus job missing CRAPTS_CORPUS_DIR pointing to runner.temp/excalidraw"
    );
    assert!(
        corpus_job
            .iter()
            .any(|l| l.trim() == "CRAPTS_CORPUS_UPDATE: \"0\""),
        "corpus job missing CRAPTS_CORPUS_UPDATE: \"0\""
    );
}

#[test]
fn corpus_workflow_is_branch_protection_compatible_and_pins_the_checkout_without_secrets_or_updates()
 {
    let root = workspace_root();
    let workflow = fs::read_to_string(root.join(".github/workflows/corpus.yml"))
        .expect("read .github/workflows/corpus.yml");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("read .github/workflows/ci.yml");
    let checkout_sh = fs::read_to_string(root.join("scripts/corpus/checkout.sh"))
        .expect("read scripts/corpus/checkout.sh");

    assert!(
        !contains_40_hex_sha(&workflow),
        "corpus.yml must not hardcode a 40-hex commit SHA"
    );
    assert!(
        !contains_40_hex_sha(&checkout_sh),
        "checkout.sh must read pin.txt instead of hardcoding a 40-hex SHA"
    );
    assert!(
        !workflow.contains("secrets."),
        "corpus.yml must not use secrets"
    );

    let wf_lines: Vec<&str> = workflow.lines().collect();
    let on_block = yaml_block(&wf_lines, "on:");
    assert!(on_block.iter().any(|l| l.trim() == "pull_request:"));
    assert!(on_block.iter().any(|l| l.trim() == "push:"));
    assert!(on_block.iter().any(|l| l.trim() == "branches: [main]"));
    assert!(on_block.iter().any(|l| l.trim() == "workflow_dispatch:"));
    assert!(
        on_block
            .iter()
            .all(|l| !l.trim().starts_with("paths:") && !l.trim().starts_with("paths-ignore:")),
        "corpus.yml must have no top-level paths filter"
    );

    let perms = yaml_block(&wf_lines, "permissions:");
    assert_eq!(
        perms
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>(),
        vec!["contents: read"]
    );

    let changes_job = yaml_block(&wf_lines, "  changes:");
    assert!(
        changes_job
            .iter()
            .any(|l| l.trim().contains("uses: dorny/paths-filter@")),
        "changes job must use dorny/paths-filter"
    );
    let corpus_globs = filter_globs(&workflow, "            corpus:");
    for expected_match in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/crapts/src/names.rs",
        "crates/cargo-crap-core/src/entry.rs",
        "crates/crapts/tests/fixtures/corpus/excalidraw/golden.json",
        "rust-toolchain.toml",
        "schemas/crapts/report-v1.json",
        "scripts/corpus/checkout.sh",
        ".gitattributes",
        ".github/workflows/corpus.yml",
    ] {
        assert!(
            any_glob_matches(&corpus_globs, expected_match),
            "corpus filter misses {expected_match}: {corpus_globs:?}"
        );
    }
    for unrelated in ["README.md", "docs/guide.md"] {
        assert!(
            !any_glob_matches(&corpus_globs, unrelated),
            "corpus filter unexpectedly matches {unrelated}"
        );
    }

    let corpus_job = yaml_block(&wf_lines, "  corpus:");
    assert_corpus_job_steps(&corpus_job);

    let ci_rust_globs = filter_globs(&ci, "            rust:");
    assert!(
        any_glob_matches(&ci_rust_globs, ".github/workflows/corpus.yml"),
        "ci.yml rust filter misses .github/workflows/corpus.yml"
    );
    assert!(
        any_glob_matches(
            &ci_rust_globs,
            "crates/crapts/tests/fixtures/corpus/excalidraw/golden.json"
        ),
        "ci.yml rust filter misses crates/*/tests/fixtures/**"
    );
}

#[test]
fn oxc_upgrade_drift_fails_until_golden_artifacts_are_updated_in_the_reviewed_diff() {
    let workflow = fs::read_to_string(workspace_root().join(".github/workflows/corpus.yml"))
        .expect("read .github/workflows/corpus.yml");
    let corpus_globs = filter_globs(&workflow, "            corpus:");
    for bumped in ["Cargo.toml", "Cargo.lock"] {
        assert!(
            any_glob_matches(&corpus_globs, bumped),
            "corpus.yml filter misses Oxc upgrade file {bumped}"
        );
    }

    let coverage = istanbul_for_a(1, 0);
    let (corpus, base_pin) = TempCorpus::new(&[("src/a.ts", base_source_a())], &coverage);
    corpus.record_baseline(&base_pin);
    let golden_json_path = corpus.fixture_dir.path().join("golden.json");
    let golden_stderr_path = corpus.fixture_dir.path().join("golden.stderr");
    let before_json = fs::read(&golden_json_path).unwrap();
    let before_stderr = fs::read(&golden_stderr_path).unwrap();

    let drifted_src = format!(
        "{}\nexport function oxc_new_branch(v: number) {{\n  return v > 0 ? v : -v;\n}}\n",
        base_source_a()
    );
    let drifted_pin = corpus.commit_file("src/a.ts", &drifted_src);

    let verify_mode = parse_update_mode(Some("0")).unwrap();
    let err = run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &drifted_pin,
        verify_mode,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert_eq!(err.kind, FailureKind::Drift);
    assert!(
        msg.contains("added row src/a.ts::oxc_new_branch") && msg.contains("outer member summary"),
        "{msg}"
    );
    assert_eq!(fs::read(&golden_json_path).unwrap(), before_json);
    assert_eq!(fs::read(&golden_stderr_path).unwrap(), before_stderr);

    let update_mode = parse_update_mode(Some("1")).unwrap();
    run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &drifted_pin,
        update_mode,
    )
    .expect("update writes refreshed golden artifacts");
    assert_ne!(fs::read(&golden_json_path).unwrap(), before_json);

    run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &drifted_pin,
        verify_mode,
    )
    .expect("subsequent verify passes after golden update");
}

#[test]
fn row_level_outer_member_duplicate_key_and_stderr_drift_fails_with_a_named_diff() {
    let coverage = istanbul_for_a(1, 0);
    let (corpus, base_pin) = TempCorpus::new(&[("src/a.ts", base_source_a())], &coverage);
    corpus.record_baseline(&base_pin);
    let baseline_golden = corpus.golden_json();
    let restored_pin = assert_source_and_coverage_drift(&corpus, &coverage);
    assert_row_and_outer_golden_drift(&corpus, &baseline_golden, &restored_pin);
    assert_stderr_order_and_duplicate_drift(&corpus, &baseline_golden, &restored_pin);
}

#[test]
fn missing_unpinned_dirty_leaking_or_invalid_corpus_inputs_fail_loudly() {
    let err = parse_corpus_dir(None).unwrap_err();
    assert_eq!(err.kind, FailureKind::MissingEnv);
    assert!(err.to_string().contains("CRAPTS_CORPUS_DIR"), "{err}");

    let err = parse_corpus_dir(Some("")).unwrap_err();
    assert_eq!(err.kind, FailureKind::MissingEnv);
    assert!(err.to_string().contains("CRAPTS_CORPUS_DIR"), "{err}");

    assert_eq!(
        parse_corpus_dir(Some("/tmp/excalidraw")).unwrap(),
        PathBuf::from("/tmp/excalidraw")
    );

    assert_eq!(parse_update_mode(None).unwrap(), CorpusMode::Verify);
    assert_eq!(parse_update_mode(Some("0")).unwrap(), CorpusMode::Verify);
    assert_eq!(parse_update_mode(Some("no")).unwrap(), CorpusMode::Verify);
    assert_eq!(parse_update_mode(Some("1")).unwrap(), CorpusMode::Update);
    let err = parse_update_mode(Some("yes")).unwrap_err();
    assert_eq!(err.kind, FailureKind::InvalidEnv);
    assert!(
        err.to_string().contains("CRAPTS_CORPUS_UPDATE") && err.to_string().contains("\"yes\""),
        "{err}"
    );

    let (corpus, head) = TempCorpus::new(&[("src/a.ts", base_source_a())], &istanbul_for_a(1, 0));
    let sentinel_json = b"sentinel-golden-json\n";
    let sentinel_stderr = b"sentinel-golden-stderr\n";
    fs::write(corpus.fixture_dir.path().join("golden.json"), sentinel_json).unwrap();
    fs::write(
        corpus.fixture_dir.path().join("golden.stderr"),
        sentinel_stderr,
    )
    .unwrap();

    let assert_fixtures_untouched = || {
        assert_eq!(
            fs::read(corpus.fixture_dir.path().join("golden.json")).unwrap(),
            sentinel_json
        );
        assert_eq!(
            fs::read(corpus.fixture_dir.path().join("golden.stderr")).unwrap(),
            sentinel_stderr
        );
    };

    let missing_dir = corpus.repo_dir.path().join("does-not-exist");
    for mode in [CorpusMode::Verify, CorpusMode::Update] {
        let err = run_corpus(&missing_dir, corpus.fixture_dir.path(), &head, mode).unwrap_err();
        assert_eq!(err.kind, FailureKind::MissingCorpusDir);
        assert!(
            err.to_string().contains(&missing_dir.display().to_string()),
            "{err}"
        );
        assert_fixtures_untouched();
    }

    let wrong_pin = "0000000000000000000000000000000000000000";
    for mode in [CorpusMode::Verify, CorpusMode::Update] {
        let err = run_corpus(
            corpus.repo_dir.path(),
            corpus.fixture_dir.path(),
            wrong_pin,
            mode,
        )
        .unwrap_err();
        assert_eq!(err.kind, FailureKind::PinMismatch);
        let msg = err.to_string();
        assert!(msg.contains(wrong_pin) && msg.contains(&head), "{msg}");
        assert_fixtures_untouched();
    }

    let extra = corpus.repo_dir.path().join("src/extra.ts");
    fs::write(&extra, "export function extra() { return 0; }\n").unwrap();
    for mode in [CorpusMode::Verify, CorpusMode::Update] {
        let err = run_corpus(
            corpus.repo_dir.path(),
            corpus.fixture_dir.path(),
            &head,
            mode,
        )
        .unwrap_err();
        assert_eq!(err.kind, FailureKind::DirtyWorktree);
        assert!(err.to_string().contains("src/extra.ts"), "{err}");
        assert_fixtures_untouched();
    }
    fs::remove_file(&extra).unwrap();

    assert_path_leak_failures(corpus.fixture_dir.path(), sentinel_json, sentinel_stderr);
}

#[test]
fn explicit_golden_update_writes_deterministic_normalized_artifacts_only_on_opt_in() {
    let broken_ts = "export function broken( {\n";
    let (corpus, head) = TempCorpus::new(
        &[("src/a.ts", base_source_a()), ("src/broken.ts", broken_ts)],
        &istanbul_for_a(1, 0),
    );
    let golden_json_path = corpus.fixture_dir.path().join("golden.json");
    let golden_stderr_path = corpus.fixture_dir.path().join("golden.stderr");
    let stale_json = b"{\"stale\": true}\n";
    let stale_stderr = b"stale stderr\n";
    fs::write(&golden_json_path, stale_json).unwrap();
    fs::write(&golden_stderr_path, stale_stderr).unwrap();

    let err = run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &head,
        CorpusMode::Verify,
    )
    .unwrap_err();
    assert_eq!(err.kind, FailureKind::Drift);
    assert_eq!(fs::read(&golden_json_path).unwrap(), stale_json);
    assert_eq!(fs::read(&golden_stderr_path).unwrap(), stale_stderr);

    run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &head,
        CorpusMode::Update,
    )
    .expect("update mode succeeds");

    let written_json = fs::read_to_string(&golden_json_path).unwrap();
    let written_stderr = fs::read_to_string(&golden_stderr_path).unwrap();
    assert!(written_json.ends_with('\n'));
    assert!(written_json.starts_with("{\n  \""));
    let parsed: Value = serde_json::from_str(&written_json).unwrap();
    assert_eq!(parsed["version"], VERSION_PLACEHOLDER);
    assert_eq!(
        parsed["$schema"],
        "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json"
    );
    assert!(
        written_stderr.contains("src/broken.ts"),
        "expected parse warning in stderr: {written_stderr:?}"
    );

    run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &head,
        CorpusMode::Verify,
    )
    .expect("verify mode passes after update");

    run_corpus(
        corpus.repo_dir.path(),
        corpus.fixture_dir.path(),
        &head,
        CorpusMode::Update,
    )
    .expect("second update succeeds");
    assert_eq!(fs::read_to_string(&golden_json_path).unwrap(), written_json);
    assert_eq!(
        fs::read_to_string(&golden_stderr_path).unwrap(),
        written_stderr
    );

    let temp_cov = corpus.fixture_dir.path().join("tmp.json");
    let scope = LeakScope {
        corpus_dir: corpus.repo_dir.path(),
        temp_coverage_path: &temp_cov,
        home: None,
    };
    let invalid_schema_err = finalize_corpus_output(
        "{\"version\":\"0.1.0\"}",
        "",
        &scope,
        corpus.fixture_dir.path(),
        CorpusMode::Update,
    )
    .unwrap_err();
    assert_eq!(invalid_schema_err.kind, FailureKind::InvalidOutput);
    assert_eq!(fs::read_to_string(&golden_json_path).unwrap(), written_json);

    let wrong_version = serde_json::to_string(&empty_valid_report("9.9.9")).unwrap();
    let wrong_version_err = finalize_corpus_output(
        &wrong_version,
        "",
        &scope,
        corpus.fixture_dir.path(),
        CorpusMode::Update,
    )
    .unwrap_err();
    assert_eq!(wrong_version_err.kind, FailureKind::InvalidOutput);
    assert!(
        wrong_version_err.to_string().contains("9.9.9"),
        "{wrong_version_err}"
    );
    assert_eq!(fs::read_to_string(&golden_json_path).unwrap(), written_json);
}

#[derive(Debug, Clone)]
struct GenRow {
    file: String,
    function: String,
    line: u64,
    cyclomatic: u64,
    coverage: f64,
    crap: f64,
    uncovered: Option<Vec<(u64, u64)>>,
    fingerprint: Option<String>,
}

impl GenRow {
    fn to_value(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("file".to_owned(), json!(self.file));
        obj.insert("function".to_owned(), json!(self.function));
        obj.insert("line".to_owned(), json!(self.line));
        obj.insert("cyclomatic".to_owned(), json!(self.cyclomatic));
        obj.insert("coverage".to_owned(), json!(self.coverage));
        obj.insert("crap".to_owned(), json!(self.crap));
        if let Some(ranges) = &self.uncovered {
            let arr: Vec<Value> = ranges
                .iter()
                .map(|(start, end)| json!({"start": start, "end": end}))
                .collect();
            obj.insert("uncovered".to_owned(), Value::Array(arr));
        }
        if let Some(fp) = &self.fingerprint {
            obj.insert("fingerprint".to_owned(), json!(fp));
        }
        Value::Object(obj)
    }
}

fn gen_row() -> impl Strategy<Value = GenRow> {
    (
        prop::sample::select(vec!["src/a.ts", "src/b.ts", "packages/x.tsx"]),
        prop::sample::select(vec!["alpha", "beta", "gamma", "delta"]),
        1u64..40,
        1u64..10,
        prop::sample::select(vec![0.0_f64, 25.0, 50.0, 75.0, 100.0]),
        prop::option::of(prop::collection::vec((1u64..20, 1u64..5), 1..3)),
        prop::option::of(prop::sample::select(vec!["a".repeat(64), "b".repeat(64)])),
    )
        .prop_map(
            |(file, function, line, cyclomatic, coverage, unc, fingerprint)| {
                let uncov = (1.0 - coverage / 100.0).powi(3);
                let crap = (cyclomatic as f64).powi(2) * uncov + (cyclomatic as f64);
                let uncovered = unc.map(|pairs| {
                    pairs
                        .into_iter()
                        .map(|(start, len)| (start, start + len))
                        .collect()
                });
                GenRow {
                    file: file.to_owned(),
                    function: function.to_owned(),
                    line,
                    cyclomatic,
                    coverage,
                    crap,
                    uncovered,
                    fingerprint,
                }
            },
        )
}

fn gen_report() -> impl Strategy<Value = Value> {
    (
        prop::sample::select(vec!["statements", "lines"]),
        prop::sample::select(vec![15.0, 30.0]),
        prop::collection::vec(prop::sample::select(vec!["n1", "n2"]), 0..3),
        prop::collection::vec(gen_row(), 0..5),
        0u64..5,
    )
        .prop_map(|(coverage_kind, threshold, notices, raw_rows, cov_files)| {
            let mut seen = BTreeSet::new();
            let mut entries = Vec::new();
            for row in raw_rows {
                let key = format!("{}::{}", row.file, row.function);
                if seen.insert(key) {
                    entries.push(row.to_value());
                }
            }
            let total = entries.len();
            json!({
                "$schema": "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v1.json",
                "version": VERSION_PLACEHOLDER,
                "name_scheme": 1,
                "coverage_kind": coverage_kind,
                "threshold": threshold,
                "selection": null,
                "notices": notices,
                "summary": {
                    "total": total,
                    "crappy": 0,
                    "regressed": null,
                    "improved": null,
                    "new": null,
                    "moved": null,
                    "unchanged": null,
                    "removed": null,
                    "changed_failures": null
                },
                "entries": entries,
                "diagnostics": {
                    "analyzed_files": 1,
                    "coverage_files": cov_files,
                    "matched_files": 1,
                    "source_only": {"count": 0, "examples": []},
                    "coverage_only": {"count": 0, "examples": []}
                }
            })
        })
}

fn reserialize_with_extra_whitespace(value: &Value) -> String {
    let compact = serde_json::to_string(value).expect("compact JSON");
    let mut spaced = String::with_capacity(compact.len() + 16);
    spaced.push_str("  \n");
    let _ = write!(spaced, "{compact}");
    spaced.push_str("  \n");
    spaced
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn property_corpus_comparator_is_symmetric_reflexive_and_format_invariant(
        a in gen_report(),
        b in gen_report(),
        stderr_a in prop::sample::select(vec!["", "warn a\n", "warn a\nwarn b\n"]),
        stderr_b in prop::sample::select(vec!["", "warn a\n", "warn a\nwarn b\n"]),
    ) {
        let ab = compare_reports(&a, &b, stderr_a, stderr_b);
        let ba = compare_reports(&b, &a, stderr_b, stderr_a);

        let equal = a == b && stderr_a == stderr_b;
        prop_assert_eq!(ab.is_ok(), equal);
        prop_assert_eq!(ab.is_ok(), ba.is_ok());

        if let (Err(err_ab), Err(err_ba)) = (&ab, &ba) {
            prop_assert_eq!(&err_ab.added_rows, &err_ba.removed_rows);
            prop_assert_eq!(&err_ab.removed_rows, &err_ba.added_rows);
        }

        prop_assert!(compare_reports(&a, &a, stderr_a, stderr_a).is_ok());

        let reparsed_compact: Value =
            serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        let reparsed_pretty: Value =
            serde_json::from_str(&serde_json::to_string_pretty(&a).unwrap()).unwrap();
        let reparsed_spaced: Value =
            serde_json::from_str(&reserialize_with_extra_whitespace(&a)).unwrap();

        for variant in [&reparsed_compact, &reparsed_pretty, &reparsed_spaced] {
            prop_assert!(compare_reports(&a, variant, stderr_a, stderr_a).is_ok());
            prop_assert_eq!(
                compare_reports(variant, &b, stderr_a, stderr_b).is_ok(),
                equal
            );
        }
    }
}
