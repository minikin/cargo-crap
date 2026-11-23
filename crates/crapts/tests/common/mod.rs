use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::LazyLock;

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

fn validator(schema: &str) -> jsonschema::Validator {
    jsonschema::validator_for(&serde_json::from_str(schema).unwrap()).unwrap()
}

static REPORT: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| validator(include_str!("../../../../schemas/crapts/report-v1.json")));
static DELTA: LazyLock<jsonschema::Validator> =
    LazyLock::new(|| validator(include_str!("../../../../schemas/crapts/delta-v1.json")));

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
