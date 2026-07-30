//! `--format sarif` — SARIF 2.1.0 output for GitHub Code Scanning, VS Code,
//! and the broader static-analysis ecosystem.
//!
//! Only crappy functions (entries above the threshold) appear as `results`.
//! The wrapping `runs` / `tool` / `driver` blocks always emit, so an empty
//! result set still produces a valid SARIF document upload-able to GitHub.

use crate::merge::CrapEntry;
use crate::score::Severity;
use anyhow::Result;
use serde::Serialize;
use std::io::Write;
use std::path::Path;

const DRIVER_NAME: &str = "cargo-crap";
const DRIVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DRIVER_INFO_URI: &str = "https://github.com/minikin/cargo-crap";
const RULE_ID: &str = "crap/high-score";
const SARIF_VERSION: &str = "2.1.0";
const SARIF_SCHEMA_URL: &str = "https://json.schemastore.org/sarif-2.1.0.json";

#[derive(Serialize)]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Serialize)]
struct SarifDriver {
    name: &'static str,
    version: &'static str,
    #[serde(rename = "informationUri")]
    information_uri: &'static str,
    rules: Vec<SarifRule>,
}

#[derive(Serialize)]
struct SarifRule {
    id: &'static str,
    #[serde(rename = "shortDescription")]
    short_description: SarifText,
    #[serde(rename = "fullDescription")]
    full_description: SarifText,
    #[serde(rename = "defaultConfiguration")]
    default_configuration: SarifLevel,
    #[serde(rename = "helpUri")]
    help_uri: &'static str,
}

#[derive(Serialize)]
struct SarifText {
    text: &'static str,
}

#[derive(Serialize)]
struct SarifLevel {
    level: &'static str,
}

#[derive(Serialize)]
struct SarifResult {
    #[serde(rename = "ruleId")]
    rule_id: &'static str,
    level: &'static str,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
}

#[derive(Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Serialize)]
struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Serialize)]
struct SarifRegion {
    #[serde(rename = "startLine")]
    start_line: usize,
}

pub(crate) fn render_sarif(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let results: Vec<SarifResult> = entries
        .iter()
        .filter(|e| Severity::classify(e.crap, threshold) == Severity::Crappy)
        .map(build_result)
        .collect();

    let log = SarifLog {
        schema: SARIF_SCHEMA_URL,
        version: SARIF_VERSION,
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: build_driver(),
            },
            results,
        }],
    };

    serde_json::to_writer_pretty(&mut *out, &log)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn build_driver() -> SarifDriver {
    SarifDriver {
        name: DRIVER_NAME,
        version: DRIVER_VERSION,
        information_uri: DRIVER_INFO_URI,
        rules: vec![SarifRule {
            id: RULE_ID,
            short_description: SarifText {
                text: "CRAP score above threshold",
            },
            full_description: SarifText {
                text: "The Change Risk Anti-Patterns (CRAP) score combines cyclomatic \
                       complexity and test coverage. Functions whose CRAP exceeds the \
                       configured threshold are flagged for refactoring or test additions.",
            },
            default_configuration: SarifLevel { level: "warning" },
            help_uri: DRIVER_INFO_URI,
        }],
    }
}

fn build_result(entry: &CrapEntry) -> SarifResult {
    SarifResult {
        rule_id: RULE_ID,
        level: "warning",
        message: SarifMessage {
            text: format_message(entry),
        },
        locations: vec![SarifLocation {
            physical_location: SarifPhysicalLocation {
                artifact_location: SarifArtifactLocation {
                    uri: normalize_path(&entry.file),
                },
                region: SarifRegion {
                    start_line: entry.line,
                },
            },
        }],
    }
}

fn format_message(entry: &CrapEntry) -> String {
    let coverage = entry
        .coverage
        .map_or_else(|| "n/a".to_string(), |c| format!("{c:.1}%"));
    format!(
        "Function `{}` has CRAP score {:.1} (cyclomatic complexity {}, coverage {})",
        entry.function, entry.crap, entry.cyclomatic as u64, coverage,
    )
}

/// Convert a path to a forward-slash URI string. SARIF's `artifactLocation.uri`
/// must use `/` regardless of host platform — Windows backslashes become `/`.
fn normalize_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
    use super::*;

    fn render_to_value(threshold: f64) -> serde_json::Value {
        let mut buf = Vec::new();
        let opts = RenderOptions {
            threshold,
            format: Format::Sarif,
            ..Default::default()
        };
        render(&sample(), &opts, &mut buf).unwrap();
        serde_json::from_slice(&buf).expect("output must be valid JSON")
    }

    #[test]
    fn sarif_output_has_schema_and_version_fields() {
        let v = render_to_value(30.0);
        assert_eq!(v["$schema"].as_str(), Some(SARIF_SCHEMA_URL));
        assert_eq!(v["version"].as_str(), Some(SARIF_VERSION));
    }

    #[test]
    fn sarif_output_has_one_run_with_a_driver() {
        let v = render_to_value(30.0);
        let runs = v["runs"].as_array().expect("runs array");
        assert_eq!(runs.len(), 1);
        let driver = &runs[0]["tool"]["driver"];
        assert_eq!(driver["name"].as_str(), Some(DRIVER_NAME));
        assert_eq!(driver["version"].as_str(), Some(DRIVER_VERSION));
        assert_eq!(driver["informationUri"].as_str(), Some(DRIVER_INFO_URI));
    }

    #[test]
    fn crappy_function_appears_as_a_result() {
        let v = render_to_value(30.0);
        let results = v["runs"][0]["results"].as_array().expect("results array");
        // sample() has one entry above the default threshold of 30 — `crappy`
        // (CRAP 110.0) — and one clean entry (CRAP 1.0).
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result["ruleId"].as_str(), Some(RULE_ID));
        assert_eq!(result["level"].as_str(), Some("warning"));
        let location = &result["locations"][0]["physicalLocation"];
        assert_eq!(
            location["artifactLocation"]["uri"].as_str(),
            Some("a.rs"),
            "uri must be the entry's file"
        );
        assert_eq!(
            location["region"]["startLine"].as_u64(),
            Some(10),
            "startLine must match the entry's line"
        );
        let message = result["message"]["text"]
            .as_str()
            .expect("message.text must be a string");
        assert!(
            message.contains("110"),
            "message must mention the CRAP score, got: {message}"
        );
        assert!(
            message.contains("crappy"),
            "message must mention the function name, got: {message}"
        );
    }

    #[test]
    fn clean_function_does_not_appear_as_a_result() {
        let v = render_to_value(30.0);
        let results = v["runs"][0]["results"].as_array().expect("results array");
        // No result should reference the clean function.
        for r in results {
            let msg = r["message"]["text"].as_str().unwrap_or("");
            assert!(!msg.contains("clean"), "clean function must not appear");
        }
    }

    #[test]
    fn empty_entries_produce_valid_sarif_with_empty_results() {
        let mut buf = Vec::new();
        render(&[], &opts(30.0, Format::Sarif), &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).expect("valid JSON");
        assert_eq!(v["version"].as_str(), Some(SARIF_VERSION));
        let results = v["runs"][0]["results"]
            .as_array()
            .expect("results array must exist even when empty");
        assert!(results.is_empty());
    }

    #[test]
    fn high_threshold_filters_all_entries() {
        // sample()'s worst entry is 110.0 — a threshold of 200 leaves nothing.
        let v = render_to_value(200.0);
        let results = v["runs"][0]["results"].as_array().expect("results array");
        assert!(results.is_empty());
    }

    #[test]
    fn windows_style_paths_are_normalized_to_forward_slashes() {
        assert_eq!(normalize_path(Path::new("src\\foo.rs")), "src/foo.rs");
        assert_eq!(
            normalize_path(Path::new("a\\b\\c.rs")),
            "a/b/c.rs",
            "every backslash must be replaced",
        );
    }

    #[test]
    fn delta_mode_with_sarif_format_returns_an_error() {
        use super::super::render_delta;
        use crate::delta::DeltaReport;
        let report = DeltaReport {
            entries: Vec::new(),
            removed: Vec::new(),
        };
        let mut buf = Vec::new();
        let err = render_delta(&report, &opts(30.0, Format::Sarif), &mut buf)
            .expect_err("delta + sarif must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("--format sarif") && msg.contains("--baseline"),
            "error must explain the incompatibility, got: {msg}"
        );
    }
}
