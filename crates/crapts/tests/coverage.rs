use crapts::coverage::{CoverageUnit, read_coverage};
use std::path::Path;

#[test]
fn read_istanbul_statement_ids_spans_and_hits_from_the_top_level_key() {
    let cwd = tempfile::tempdir().unwrap();
    let path = Path::new("report.data");
    let report = include_str!("fixtures/coverage/statements.json");
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
    let report = include_str!("fixtures/coverage/wide.json");
    std::fs::write(&path, report).unwrap();
    assert_eq!(
        read_coverage(cwd.path(), &path).unwrap().keys(),
        ["src/a.ts"]
    );
    std::fs::write(&path, report.replace("1e300", "-0")).unwrap();
    assert_eq!(
        read_coverage(cwd.path(), &path).unwrap().unit(),
        CoverageUnit::Statements
    );
    std::fs::write(&path, report.replace("9007199254740993", "-0")).unwrap();
    assert!(
        read_coverage(cwd.path(), &path)
            .unwrap_err()
            .to_string()
            .ends_with("statement x: invalid start")
    );
    std::fs::write(&path, report.replace("1e300", "1e400")).unwrap();
    assert!(
        read_coverage(cwd.path(), &path)
            .unwrap_err()
            .to_string()
            .ends_with("src/a.ts: statement x: invalid hit count")
    );
}

#[test]
fn detect_the_coverage_unit_from_the_contents() {
    use CoverageUnit::{Lines, Statements};
    let cwd = tempfile::tempdir().unwrap();
    let cases: &[(&str, &[u8], CoverageUnit, &[&str])] = &[
        ("run.lcov", b"\xef\xbb\xbf \t\n{}", Statements, &[]),
        (
            "run.json",
            b"\xef\xbb\xbfSF:src/a.ts\r\n",
            Lines,
            &["src/a.ts"],
        ),
        (
            "run.data",
            b"TN:suite\nSF:./src/a.ts\nend_of_record",
            Lines,
            &["./src/a.ts"],
        ),
        (
            "interior",
            "SF:src/a\u{feff}b.ts".as_bytes(),
            Lines,
            &["src/a\u{feff}b.ts"],
        ),
        ("lossy", b"SF:src/a\xffb.ts", Lines, &["src/a\u{fffd}b.ts"]),
        ("spaced", " \u{feff}{}".as_bytes(), Statements, &[]),
    ];
    for &(name, bytes, unit, keys) in cases {
        std::fs::write(cwd.path().join(name), bytes).unwrap();
        let report = read_coverage(cwd.path(), &Path::new("absent").join("..").join(name)).unwrap();
        assert_eq!(report.keys(), keys, "{name}");
        assert_eq!(report.unit(), unit, "{name}");
    }
}

#[test]
fn identify_an_unreadable_or_unrecognized_report() {
    let cwd = tempfile::tempdir().unwrap();
    let directory = cwd.path().join("reports");
    std::fs::create_dir(&directory).unwrap();
    for (path, code) in [
        (directory, "EISDIR"),
        (cwd.path().join("gone.info"), "ENOENT"),
    ] {
        let error = read_coverage(cwd.path(), path.file_name().unwrap().as_ref()).unwrap_err();
        assert_eq!(error.path(), path);
        assert_eq!(error.to_string(), format!("{}: {code}", path.display()));
    }
    let cases = [
        ("control", "\u{85}{}", "unrecognized coverage format"),
        ("broken", "{\nSF:a.ts", "invalid JSON"),
        ("empty", "", "unrecognized coverage format"),
        ("array", "[]", "unrecognized coverage format"),
        (
            "text",
            "TN:prefix SF:x\nDA:1,1",
            "unrecognized coverage format",
        ),
        ("indented", "  SF:a.ts", "unrecognized coverage format"),
        (
            "double-bom",
            "\u{feff}\u{feff}SF:a.ts",
            "unrecognized coverage format",
        ),
        (
            "bad.lcov",
            "TN:x\nDA:bad\nSF:a.ts",
            "line 2: DA outside an SF record",
        ),
    ];
    for (name, bytes, detail) in cases {
        let path = cwd.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let error = read_coverage(cwd.path(), Path::new(name)).unwrap_err();
        assert_eq!(error.path(), path);
        assert!(
            error
                .to_string()
                .starts_with(&format!("{}: {detail}", path.display())),
            "{error}"
        );
    }
}

#[test]
fn report_labels_resolve_parent_components_from_the_callers_directory() {
    let root = tempfile::tempdir().unwrap();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let target = root.path().join("r.json");
    let error = read_coverage(&child, Path::new("../r.json")).unwrap_err();
    assert_eq!(error.path().as_os_str(), target.as_os_str());
    assert_eq!(error.to_string(), format!("{}: ENOENT", target.display()));
}
