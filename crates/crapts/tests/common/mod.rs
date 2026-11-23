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

/// Picks the schema from the document's own `$schema`, so a delta report can
/// never pass as an absolute one.
pub fn validated(bytes: &[u8]) -> Value {
    let doc: Value = serde_json::from_slice(bytes).unwrap();
    let schema = doc["$schema"].as_str().unwrap_or_default();
    let validator = if schema.ends_with("/crapts/delta-v1.json") {
        &*DELTA
    } else {
        assert!(schema.ends_with("/crapts/report-v1.json"), "{schema}");
        &*REPORT
    };
    if let Err(error) = validator.validate(&doc) {
        panic!("{error}: {doc:#}");
    }
    doc
}
