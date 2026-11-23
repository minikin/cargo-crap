//! A caller that links cargo-crap-core and nothing of cargo-crap.

use cargo_crap_core::delta::DEFAULT_EPSILON;
use cargo_crap_core::entry::{CrapEntry, SortOrder};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::policy::{
    EnvelopeKind, NoticeSpacing, PairPolicy, Policy, Population, TieOrder,
};
use cargo_crap_core::report::{Format, Schemas, Tool};
use cargo_crap_core::run::{self, Analysis, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

const CARGO_CRAP_FIXTURE: Tool = include!("../src/report/cargo_crap_tool.inc");
const SECOND_TOOL: Tool = include!("../src/report/second_tool.inc");

struct OnlyFile(&'static str);

impl Scope for OnlyFile {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        file == Path::new(self.0)
    }
}

#[derive(Default)]
struct Recorder {
    written: Vec<u8>,
    flushed: Option<usize>,
}

impl Write for Recorder {
    fn write(
        &mut self,
        buf: &[u8],
    ) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed = Some(self.written.len());
        Ok(())
    }
}

#[test]
fn a_core_only_caller_can_report_a_scored_function() {
    // Given spec 46's SECOND_TOOL, version "7.7.7-other", with report schema
    // "https://example.test/schemas/other-report.json"
    assert_eq!(SECOND_TOOL.version, "7.7.7-other");
    assert_eq!(
        SECOND_TOOL.schemas.report,
        "https://example.test/schemas/other-report.json"
    );
    assert_eq!(SECOND_TOOL.policy.fail_above_counts, Population::Shown);
    // And one eligible row for "f" at src/a.rs:3 with complexity 2,
    // coverage 50% and CRAP 2.5
    let row = CrapEntry {
        file: "src/a.rs".into(),
        function: "f".into(),
        line: 3,
        cyclomatic: 2.0,
        coverage: Some(50.0),
        crap: 2.5,
        ..CrapEntry::default()
    };
    // A second crappy row that `top` hides: the Shown population counts one
    // row where All would count two.
    let hidden = CrapEntry {
        file: "src/a.rs".into(),
        function: "g".into(),
        line: 9,
        cyclomatic: 2.0,
        coverage: Some(60.0),
        crap: 2.256,
        ..CrapEntry::default()
    };
    let allow = AllowSet::compile(&[]).expect("an empty allow set compiles");
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: Some(1),
        sort: SortOrder::Crap,
        threshold: 2.0,
        epsilon: DEFAULT_EPSILON,
        signals: &signals,
        fail_above: true,
        fail_regression: false,
        format: Format::Json,
        summary: false,
        show_unchanged: false,
        uncovered_hints: false,
        width: None,
        links: None,
        cwd: Path::new("."),
        workspace_root: None,
        pr_comment_marker: None,
        repo_prefix: None,
        fail_sections: false,
    };
    let analysis = Analysis {
        entries: vec![row, hidden],
        diagnostics: None,
        scope: &OnlyFile("src/a.rs"),
        coverage_kind: None,
        notices: &[],
    };

    // When it calls run::run with JSON output, threshold 2, fail-above
    // enabled, Shown policy and no baseline
    let mut out = Recorder::default();
    let outcome = run::run(
        &SECOND_TOOL,
        &settings,
        analysis,
        None,
        &[],
        &RenderExtras::default(),
        &mut out,
    )
    .expect("the report renders");

    // Then the JSON has that version and schema, and contains the row for
    // "f" at CRAP 2.5
    let json: serde_json::Value = serde_json::from_slice(&out.written).expect("JSON output");
    assert_eq!(json["version"], "7.7.7-other");
    assert_eq!(
        json["$schema"],
        "https://example.test/schemas/other-report.json"
    );
    let entries = json["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1, "{json:#}");
    assert_eq!(entries[0]["function"], "f");
    assert_eq!(entries[0]["file"], "src/a.rs");
    assert_eq!(entries[0]["line"], 3);
    assert_eq!(entries[0]["cyclomatic"], 2.0);
    assert_eq!(entries[0]["coverage"], 50.0);
    assert_eq!(entries[0]["crap"], 2.5);
    // And the flushed report is followed by an outcome with crappy = 1 and
    // exit code 1
    assert_eq!(out.flushed, Some(out.written.len()));
    assert_eq!(outcome.crappy, 1);
    assert_eq!(outcome.exit_code(), ExitCode::from(1));
}

mod coverage_spelling {
    use super::Path;
    use cargo_crap_core::entry::StrayFiles;
    use cargo_crap_core::paths::{Assignment, TiePolicy, assign_by_key};

    fn files(sources: &[&str]) -> tempfile::TempDir {
        let cwd = tempfile::tempdir().unwrap();
        for name in sources {
            let file = cwd.path().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "function f() { return 1 }").unwrap();
        }
        cwd
    }

    fn assign(
        cwd: &Path,
        sources: &[&str],
        keys: &[&str],
    ) -> Assignment {
        assign_by_key(cwd, sources, keys, TiePolicy::Unmatched)
    }

    #[test]
    fn match_whole_path_components_across_checkout_and_platform_spellings() {
        let cases: &[(&[&str], &str, Option<&str>)] = &[
            (&["pkg/src/a.ts"], r".\src\a.ts", Some("pkg/src/a.ts")),
            (
                &["src/a.ts", "vendor/src/a.ts"],
                "src/a.ts",
                Some("src/a.ts"),
            ),
            (&["src/a.ts"], "work/src/a.ts", None),
            (&["src/a.ts"], "/ci/work/src/a.ts", Some("src/a.ts")),
            (
                &["a.ts", "src/a.ts"],
                r"D:\ci\work\src\a.ts",
                Some("src/a.ts"),
            ),
            (&["src/a.ts"], "/ci/osrc/a.ts", None),
            (&["oofoo/bar.ts"], "foo/bar.ts", None),
            (&["src/a.ts"], "src/A.ts", None),
            (&["src/a.ts"], "./", None),
        ];
        for &(sources, key, owner) in cases {
            let cwd = files(sources);
            let result = assign(cwd.path(), sources, &[key]);
            for (&source, binding) in sources.iter().zip(result.bindings) {
                let expected: Vec<_> = (Some(source) == owner).then_some(0).into_iter().collect();
                assert_eq!(binding, expected, "{source} with {key}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_absolute_alias_resolves_through_the_filesystem_and_a_relative_alias_stays_lexical() {
        let sources = ["lib/real.ts"];
        let cwd = files(&sources);
        let alias = cwd.path().join("alias.ts");
        std::os::unix::fs::symlink(cwd.path().join(sources[0]), &alias).unwrap();
        let keys = ["alias.ts", alias.to_str().unwrap()];
        assert_eq!(
            assign(cwd.path(), &sources, &keys[..1]).bindings,
            [Vec::<usize>::new()]
        );
        assert_eq!(assign(cwd.path(), &sources, &keys[1..]).bindings, [vec![0]]);
    }

    #[test]
    fn a_real_path_group_takes_precedence_over_every_lexical_key() {
        let sources = ["src/a.ts"];
        let cwd = files(&sources);
        let absolute = cwd
            .path()
            .join(sources[0])
            .to_str()
            .unwrap()
            .replace('/', "\\");
        let keys = ["a.ts", sources[0], &absolute];
        for (keys, expected) in [(&keys[..], 2), (&keys[..2], 1)] {
            assert_eq!(
                assign(cwd.path(), &sources, keys).bindings,
                [vec![expected]]
            );
        }
    }

    #[test]
    fn preserve_a_shared_key_tie_even_when_one_source_has_better_coverage() {
        let sources = ["packages/b/src/index.ts", "packages/a/src/index.ts"];
        let cwd = files(&sources);
        let absolute = cwd.path().join(sources[1]);
        for extra in [
            None,
            Some("a/src/index.ts"),
            Some(sources[1]),
            absolute.to_str(),
        ] {
            let mut keys = vec!["src/index.ts"];
            keys.extend(extra);
            let result = assign(cwd.path(), &sources, &keys);
            let expected: Vec<usize> = extra.map(|_| 1).into_iter().collect();
            assert_eq!(result.bindings, [vec![], expected]);
            assert_eq!(result.ties.len(), 1);
            assert_eq!(result.ties[0].keys, [0]);
            let mut owners = result.ties[0].sources.clone();
            owners.sort_unstable();
            assert_eq!(owners, [0, 1]);
        }
    }

    #[test]
    fn stray_examples_sort_original_strings_before_taking_the_first_ten() {
        let pair = StrayFiles::from_raw_spellings(&["a/b.ts", "a-b/c.ts"]);
        assert_eq!(
            pair.examples
                .iter()
                .map(|p| p.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["a-b/c.ts", "a/b.ts"]
        );
        let mut names = vec!["a/b.ts".to_owned(), "a-b/c.ts".to_owned()];
        names.extend((0..9).rev().map(|i| format!("{i:02}.ts")));
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let stray = StrayFiles::from_raw_spellings(&refs);
        assert_eq!(stray.count, 11);
        let mut expected: Vec<_> = (0..9).map(|i| format!("{i:02}.ts")).collect();
        expected.push("a-b/c.ts".to_owned());
        assert_eq!(
            stray
                .examples
                .iter()
                .map(|p| p.to_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn diagnostics_retain_separator_aliases_and_sort_supplementary_characters_by_utf16() {
        let names = [
            "src/a.ts",
            "src/./a.ts",
            r"src\a.ts",
            "\u{e000}.ts",
            "\u{10000}.ts",
        ];
        let stray = StrayFiles::from_raw_spellings(&names);
        assert_eq!(stray.count, 5);
        assert_eq!(
            stray
                .examples
                .iter()
                .map(|p| p.to_str().unwrap())
                .collect::<Vec<_>>(),
            [names[1], names[0], names[2], names[4], names[3]]
        );
    }
}

#[test]
fn resume_strict_lcov_counts_across_records_and_ignore_unrelated_records() {
    use cargo_crap_core::coverage::{StrictLcovFile, read_strict_lcov};
    let report = "TN:suite\r\nSF:./src/a.ts\r\nDA:4,0\r\nDA:4,2,checksum\r\nFN:not,a,proper,record\r\nSF:src/b.ts\r\nDA:4,7\r\nend_of_record\r\nSF:./src/a.ts\r\nDA:4,3\r\nSF:empty.ts";
    let files: Vec<StrictLcovFile> = read_strict_lcov(report, Path::new("cov.info")).unwrap();
    let keys: Vec<_> = files.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["./src/a.ts", "src/b.ts", "empty.ts"]);
    assert_eq!(files[0].lines[&4], 5.0);
    assert_eq!(files[1].lines[&4], 7.0);
    assert_eq!(files[2].lines.len(), 0);
}

#[test]
fn strict_lcov_errors_identify_the_physical_record() {
    use cargo_crap_core::coverage::read_strict_lcov;
    let cases = [
        ("TN:x\nDA:8,1\nSF:a.ts", 2, "DA outside an SF record"),
        ("TN:x\nDA:malformed\nSF:a.ts", 2, "DA outside an SF record"),
        (
            "SF:a.ts\nend_of_record\nDA:8,1",
            3,
            "DA outside an SF record",
        ),
        ("SF:a.ts\nSF:", 2, "missing SF path"),
        ("SF:a.ts\nDA:0,1", 2, "invalid DA number"),
        ("SF:a.ts\nDA:9007199254740992,1", 2, "invalid DA number"),
        ("SF:a.ts\nDA:+1,+2", 2, "invalid DA record"),
        ("SF:a.ts\nDA:1,-1", 2, "invalid DA record"),
        ("SF:a.ts\nDA:1,1.5", 2, "invalid DA record"),
        ("SF:a.ts\nDA:1,1,", 2, "invalid DA record"),
        ("SF:a.ts\nDA:1,1,checksum,extra", 2, "invalid DA record"),
    ];
    for (text, line, reason) in cases {
        let error = read_strict_lcov(text, Path::new("cov.info")).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("cov.info: line {line}: {reason}"),
            "{text}"
        );
    }
}

#[test]
fn read_strict_lcov_at_its_numeric_boundaries() {
    use cargo_crap_core::coverage::read_strict_lcov;
    let report = "SF:a.ts\nDA:4294967296,1\nDA:9007199254740991,0\nDA:1,18446744073709551616";
    let files = read_strict_lcov(report, Path::new("cov.info")).unwrap();
    assert_eq!(
        files[0].lines,
        [
            (1, 2_f64.powi(64)),
            (4_294_967_296, 1.0),
            (9_007_199_254_740_991, 0.0)
        ]
        .into()
    );
    let huge = format!("1{}", "0".repeat(308));
    let overflow = format!("SF:a\nDA:2,{huge}\nDA:2,{huge}");
    let error = read_strict_lcov(&overflow, Path::new("cov.info")).unwrap_err();
    assert_eq!(error.to_string(), "cov.info: line 3: invalid DA number");
}

#[test]
fn a_caller_can_find_a_parent_config_and_apply_its_allows_to_missing_paths() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("packages/app");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(root.path().join(".example"), "pack*/app/old/**").unwrap();
    let config = cargo_crap_core::config::find_upwards(&cwd, ".example", Some(root.path()))
        .unwrap()
        .unwrap();
    let pattern = std::fs::read_to_string(&config).unwrap();
    let set = AllowSet::compile_with_bases(&cwd, &[(&pattern, config.parent().unwrap())]).unwrap();
    let mut rows = vec![CrapEntry {
        file: "old/missing.ts".into(),
        ..CrapEntry::default()
    }];
    cargo_crap_core::filter::allow(&mut rows, &set);
    assert!(rows.is_empty());
}

#[test]
fn repository_paths_are_projected_only_for_link_and_annotation_destinations() {
    use cargo_crap_core::report::{RenderOptions, SourceLinks, render};
    let links = SourceLinks::new("https://example.test/o/r".into(), "abc".into());
    let row = CrapEntry {
        file: "../shared/a,b #1.ts".into(),
        function: "f%2Fg".into(),
        line: 2,
        crap: 42.0,
        ..CrapEntry::default()
    };
    for format in [
        Format::Markdown,
        Format::PrComment,
        Format::GitHub,
        Format::Sarif,
        Format::Json,
    ] {
        let options = RenderOptions {
            format,
            links: Some(&links),
            repo_prefix: Some(Path::new("packages/app")),
            pr_comment_marker: Some("<!-- local -->"),
            ..RenderOptions::new(&CARGO_CRAP_FIXTURE)
        };
        let mut out = Vec::new();
        render(std::slice::from_ref(&row), &options, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        match format {
            Format::Markdown | Format::PrComment => {
                let marker = if format == Format::Markdown {
                    CARGO_CRAP_FIXTURE.comment_marker
                } else {
                    "<!-- local -->"
                };
                assert_eq!(text.lines().next(), Some(marker));
                assert_eq!(
                    text.matches(
                        "https://example.test/o/r/blob/abc/packages/shared/a%2Cb%20%231.ts#L2"
                    )
                    .count(),
                    2,
                    "{text}"
                );
            },
            Format::GitHub => {
                assert!(
                    text.contains("file=packages/shared/a%2Cb #1.ts,line=2"),
                    "{text}"
                );
                assert!(text.contains("f%252Fg"), "{text}");
            },
            Format::Sarif => {
                let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    doc["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
                        ["uri"],
                    "packages/shared/a,b #1.ts"
                );
            },
            Format::Json => {
                let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(doc["entries"][0]["file"], "../shared/a,b #1.ts");
            },
            _ => unreachable!(),
        }
    }
}
