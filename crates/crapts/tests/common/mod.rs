#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;

const PUBLISHED: &str = "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/";

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Serves published schema URLs from this checkout, so a test needs no
/// network and checks the schemas being changed rather than those on main.
struct LocalSchemas;

impl jsonschema::Retrieve for LocalSchemas {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        let file = uri
            .as_str()
            .strip_prefix(PUBLISHED)
            .ok_or_else(|| format!("not a published schema: {}", uri.as_str()))?;
        let path = repository_root().join("schemas").join(file);
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

pub fn compile(path: &str) -> jsonschema::Validator {
    let raw = std::fs::read_to_string(repository_root().join(path))
        .unwrap_or_else(|e| panic!("read {path}: {e}"));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("schema must be valid JSON");
    jsonschema::options()
        .with_retriever(LocalSchemas)
        .build(&value)
        .unwrap_or_else(|e| panic!("{path} must compile: {e}"))
}

pub fn binary(cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crapts"));
    cmd.current_dir(cwd);
    for key in [
        "GITHUB_REPOSITORY",
        "GITHUB_SERVER_URL",
        "GITHUB_SHA",
        "NO_COLOR",
        "FORCE_COLOR",
        "COLUMNS",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

pub fn invoke(
    cwd: &Path,
    args: &[&str],
) -> Output {
    binary(cwd).args(args).output().unwrap()
}

pub fn check(
    out: &Output,
    code: i32,
) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

static REPORT: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| compile("schemas/crapts/report-v1.json"));
static DELTA: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| compile("schemas/crapts/delta-v1.json"));

fn validated(
    bytes: &[u8],
    validator: &jsonschema::Validator,
    file: &str,
) -> Value {
    let doc: Value = serde_json::from_slice(bytes).unwrap();
    let schema = doc["$schema"].as_str().unwrap_or_default();
    assert!(schema.ends_with(file), "{schema}");
    if let Err(error) = validator.validate(&doc) {
        panic!("{error}: {doc:#}");
    }
    doc
}

pub fn validated_report(bytes: &[u8]) -> Value {
    validated(bytes, &REPORT, "/crapts/report-v1.json")
}

pub fn validated_delta(bytes: &[u8]) -> Value {
    validated(bytes, &DELTA, "/crapts/delta-v1.json")
}
