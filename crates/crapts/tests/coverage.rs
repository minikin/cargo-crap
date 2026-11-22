use crapts::coverage::{CoverageUnit, read_coverage};
use std::path::Path;

#[test]
fn read_istanbul_statement_ids_spans_and_hits_from_the_top_level_key() {
    let cwd = tempfile::tempdir().unwrap();
    let path = Path::new("report.data");
    let report = r#"{"./src/a.ts":{"path":"ignored.ts","fnMap":{},"b":{},"s":{"x":7},
        "statementMap":{"x":{"start":{"line":2,"column":9},"end":{"line":3,"column":0}}}},
        "empty.ts":{"statementMap":{},"s":{}}}"#;
    std::fs::write(cwd.path().join(path), report).unwrap();
    let coverage = read_coverage(cwd.path(), path).unwrap();
    assert_eq!(coverage.unit(), CoverageUnit::Statements);
    assert_eq!(coverage.keys(), ["./src/a.ts", "empty.ts"]);
}

#[test]
fn reject_malformed_istanbul_objects_endpoints_and_counts() {
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().join("run.json");
    std::fs::write(
        &path,
        r#"{"src/a.ts":{"statementMap":{"x":false},"s":{"x":false,"extra":0}}}"#,
    )
    .unwrap();
    let error = read_coverage(cwd.path(), Path::new("run.json")).unwrap_err();
    assert_eq!(error.path(), path);
    assert_eq!(
        error.to_string(),
        format!("{}: src/a.ts: statement extra has no span", path.display())
    );
}

#[test]
fn duplicate_json_members_take_their_final_value_at_their_first_position() {
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().join("duplicate");
    std::fs::write(
        &path,
        r#"{"x.ts":false,"./x.ts":{"statementMap":{},"s":{}},"x.ts":{"statementMap":{},"s":{}}}"#,
    )
    .unwrap();
    assert_eq!(
        read_coverage(cwd.path(), &path).unwrap().keys(),
        ["x.ts", "./x.ts"]
    );
}

#[test]
fn istanbul_uses_binary64_integers_for_positions_and_hits() {
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().join("wide");
    let report = r#"{"file":{"statementMap":{"x":{"start":{"line":9007199254740993,"column":-0},"end":{"line":9007199254740994,"column":null}}},"s":{"x":1e300}}}"#;
    std::fs::write(&path, report).unwrap();
    assert_eq!(read_coverage(cwd.path(), &path).unwrap().keys(), ["file"]);
    std::fs::write(&path, report.replace("1e300", "1e400")).unwrap();
    assert!(
        read_coverage(cwd.path(), &path)
            .unwrap_err()
            .to_string()
            .ends_with("file: statement x: invalid hit count")
    );
}
