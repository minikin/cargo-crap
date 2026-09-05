//! `--format json` and `--format json --baseline …` envelope output.
//!
//! Outputs a versioned, schema-tagged envelope so consumers can detect
//! breaking changes between releases. The envelope shape is mirrored on
//! input as well — `delta::load_baseline` deserializes the same struct.

use crate::delta::{DeltaEntry, DeltaReport};
use crate::duplicates::compare::DuplicatePair;
use crate::merge::{CrapEntry, ScopeDiagnostics};
use anyhow::Result;
use std::io::Write;

/// Schema/release version stamped onto every JSON envelope so consumers can
/// detect breaking changes between releases. Mirrors the crate version.
pub const SCHEMA_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build the published HTTPS URL for a schema file in this repo.
///
/// `concat!` only takes literals, so the base URL is repeated by expansion
/// rather than reference. Centralized here so a repo move or schema-version
/// bump changes only this macro and the filename arguments below.
macro_rules! schema_url {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/",
            $file
        )
    };
}

/// Stable HTTPS URL of the JSON Schema describing the absolute envelope shape.
pub const REPORT_SCHEMA_URL: &str = schema_url!("report-v1.json");

/// Stable HTTPS URL of the JSON Schema describing the delta envelope shape.
///
/// Bumped to `delta-v2.json` in spec 13: adds the `moved` status value and
/// the optional `previous_file` field. Consumers reading v1 see one new
/// enum value and one new optional field — strictly additive.
pub const DELTA_SCHEMA_URL: &str = schema_url!("delta-v2.json");

/// JSON wire format for `--format json` output and `--baseline` input.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Envelope {
    /// URL of the JSON Schema this document conforms to. Optional on input
    /// (older baselines may predate the field) and always emitted on output.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub version: String,
    pub entries: Vec<CrapEntry>,
    /// Source/LCOV scope diagnostics (spec 24). Present only when the run
    /// had an `--lcov` input; ignored when the envelope is read back as a
    /// `--baseline` (the mismatch is a property of the producing run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<ScopeDiagnostics>,
    /// Candidate duplicate pairs. Present only when duplicate detection was
    /// requested; absent — not empty — otherwise, so a baseline written by an
    /// older version still reads, and one written without `--duplicates`
    /// does not claim there were none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicates: Option<Vec<DuplicateJson>>,
}

/// One candidate duplicate pair, flattened for the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DuplicateJson {
    /// Path of the side that sorts first.
    pub first_file: String,
    /// Its function or method name.
    pub first_function: String,
    /// Its first line, 1-indexed and inclusive.
    pub first_start_line: usize,
    /// Its last line, 1-indexed and inclusive.
    pub first_end_line: usize,
    /// Path of the side that sorts second.
    pub second_file: String,
    /// Its function or method name.
    pub second_function: String,
    /// Its first line, 1-indexed and inclusive.
    pub second_start_line: usize,
    /// Its last line, 1-indexed and inclusive.
    pub second_end_line: usize,
    /// Jaccard similarity of the two fingerprint sets.
    pub score: f64,
}

impl DuplicateJson {
    /// Flatten a pair for serialization.
    #[must_use]
    pub fn from_pair(pair: &DuplicatePair) -> Self {
        Self {
            first_file: pair.first.file.display().to_string(),
            first_function: pair.first.name.clone(),
            first_start_line: pair.first.start_line,
            first_end_line: pair.first.end_line,
            second_file: pair.second.file.display().to_string(),
            second_function: pair.second.name.clone(),
            second_start_line: pair.second.start_line,
            second_end_line: pair.second.end_line,
            score: pair.score,
        }
    }
}

/// Flatten pairs for either envelope, preserving their order.
fn wire(pairs: Option<&[DuplicatePair]>) -> Option<Vec<DuplicateJson>> {
    pairs.map(|ps| ps.iter().map(DuplicateJson::from_pair).collect())
}

pub(crate) fn render_json(
    entries: &[CrapEntry],
    diagnostics: Option<&ScopeDiagnostics>,
    duplicates: Option<&[DuplicatePair]>,
    out: &mut dyn Write,
) -> Result<()> {
    let envelope = Envelope {
        schema: Some(REPORT_SCHEMA_URL.to_string()),
        version: SCHEMA_VERSION.to_string(),
        entries: entries.to_vec(),
        diagnostics: diagnostics.cloned(),
        duplicates: wire(duplicates),
    };
    serde_json::to_writer_pretty(&mut *out, &envelope)?;
    out.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn render_delta_json(
    report: &DeltaReport,
    diagnostics: Option<&ScopeDiagnostics>,
    duplicates: Option<&[DuplicatePair]>,
    out: &mut dyn Write,
) -> Result<()> {
    #[derive(serde::Serialize)]
    struct DeltaOutput<'a> {
        #[serde(rename = "$schema")]
        schema: &'static str,
        version: &'static str,
        entries: &'a [DeltaEntry],
        removed: &'a [crate::delta::RemovedEntry],
        #[serde(skip_serializing_if = "Option::is_none")]
        diagnostics: Option<&'a ScopeDiagnostics>,
        // Carried in delta mode too: dropping them here would lose data the
        // run was explicitly asked for, silently.
        #[serde(skip_serializing_if = "Option::is_none")]
        duplicates: Option<Vec<DuplicateJson>>,
    }
    serde_json::to_writer_pretty(
        &mut *out,
        &DeltaOutput {
            schema: DELTA_SCHEMA_URL,
            version: SCHEMA_VERSION,
            entries: &report.entries,
            removed: &report.removed,
            diagnostics,
            duplicates: wire(duplicates),
        },
    )?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{opts, sample};
    use super::super::{Format, RenderOptions, render};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn json_output_is_envelope_with_version_and_entries() {
        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Json), &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(parsed.is_object(), "JSON output must be an envelope object");
        assert_eq!(
            parsed["version"].as_str(),
            Some(SCHEMA_VERSION),
            "version field must equal SCHEMA_VERSION"
        );
        assert!(
            parsed["entries"].is_array(),
            "entries field must be an array"
        );
        assert_eq!(
            parsed["entries"].as_array().map(std::vec::Vec::len),
            Some(2)
        );
    }

    #[test]
    fn diagnostics_embedded_when_present_and_absent_otherwise() {
        use crate::merge::StrayFiles;
        let diag = ScopeDiagnostics {
            analyzed_files: 4,
            lcov_files: 3,
            matched_files: 2,
            source_only: StrayFiles {
                count: 2,
                examples: vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
            },
            lcov_only: StrayFiles {
                count: 1,
                examples: vec![PathBuf::from("src/gone.rs")],
            },
        };

        let mut buf = Vec::new();
        render(
            &sample(),
            &RenderOptions {
                threshold: 30.0,
                format: Format::Json,
                diagnostics: Some(&diag),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed["diagnostics"]["analyzed_files"], 4);
        assert_eq!(parsed["diagnostics"]["lcov_files"], 3);
        assert_eq!(parsed["diagnostics"]["matched_files"], 2);
        assert_eq!(parsed["diagnostics"]["source_only"]["count"], 2);
        assert_eq!(
            parsed["diagnostics"]["source_only"]["examples"][0],
            "src/a.rs"
        );
        assert_eq!(parsed["diagnostics"]["lcov_only"]["count"], 1);

        let mut buf = Vec::new();
        render(&sample(), &opts(30.0, Format::Json), &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(
            parsed.get("diagnostics").is_none(),
            "no diagnostics → no key in the envelope"
        );
    }

    #[test]
    fn json_format_unaffected_by_links() {
        use super::super::SourceLinks;
        let entries = vec![CrapEntry {
            file: PathBuf::from("src/a.rs"),
            function: "foo".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
            uncovered: Vec::new(),
        }];
        let links = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let mut buf = Vec::new();
        render(
            &entries,
            &RenderOptions {
                threshold: 30.0,
                format: Format::Json,
                links: Some(&links),
                ..Default::default()
            },
            &mut buf,
        )
        .unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            !s.contains("](https://"),
            "JSON output must not contain markdown links:\n{s}"
        );
    }
}
