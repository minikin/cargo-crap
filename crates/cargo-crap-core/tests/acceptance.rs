//! A caller that links cargo-crap-core and nothing of cargo-crap.

use cargo_crap_core::delta::DEFAULT_EPSILON;
use cargo_crap_core::entry::{CrapEntry, SortOrder};
use cargo_crap_core::filter::AllowSet;
use cargo_crap_core::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use cargo_crap_core::report::{Format, Schemas, Tool};
use cargo_crap_core::run::{self, Analysis, RenderExtras, Scope, Settings};
use cargo_crap_core::signals::SignalSettings;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

const CARGO_CRAP_FIXTURE: Tool = include!("fixtures/cargo_crap_tool.inc.rs");
const SECOND_TOOL: Tool = include!("fixtures/second_tool.inc.rs");

struct OnlyFile(&'static str);

impl Scope for OnlyFile {
    fn keeps(
        &self,
        file: &Path,
    ) -> bool {
        file == Path::new(self.0)
    }
}

/// Keeps the bytes written and how many of them the last flush covered.
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
    let allow = AllowSet::compile(&[]).expect("an empty allow set compiles");
    let signals = SignalSettings::default();
    let settings = Settings {
        allow: &allow,
        min: None,
        top: None,
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
    };
    let analysis = Analysis {
        entries: vec![row],
        diagnostics: None,
        scope: &OnlyFile("src/a.rs"),
    };

    // When it calls run::run with JSON output, threshold 2, fail-above
    // enabled, Shown policy and no baseline
    let mut out = Recorder::default();
    let outcome = run::run(
        &SECOND_TOOL,
        &settings,
        analysis,
        None,
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
    assert_eq!(entries[0]["crap"], 2.5);
    // And the flushed report is followed by an outcome with crappy = 1 and
    // exit code 1
    assert_eq!(out.flushed, Some(out.written.len()));
    assert_eq!(outcome.crappy, 1);
    assert_eq!(outcome.exit_code(), ExitCode::from(1));
}
