//! `--format shields` — Shields.io endpoint-badge JSON (spec 15).
//!
//! Emits the single JSON object consumed by Shields.io's
//! [endpoint badge](https://shields.io/badges/endpoint-badge). The badge
//! reports how many functions exceed the CRAP threshold. There is no delta
//! variant: when `--baseline` is supplied the flag is silently ignored and
//! the badge reflects absolute current scores only.

use crate::delta::DeltaReport;
use crate::merge::CrapEntry;
use crate::score::Severity;
use anyhow::Result;
use serde::Serialize;
use std::io::Write;

/// Shields.io endpoint schema version — always 1 per the published schema.
const SCHEMA_VERSION: u32 = 1;

/// The [Shields.io endpoint schema v1](https://shields.io/endpoint) object.
#[derive(Serialize)]
struct Badge {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    label: String,
    message: String,
    color: &'static str,
}

/// Format the threshold for the badge label without trailing zeros:
/// `15`, not `15.0` — but `12.5` keeps its fraction.
fn format_threshold(threshold: f64) -> String {
    if threshold.fract() == 0.0 {
        format!("{threshold:.0}")
    } else {
        format!("{threshold}")
    }
}

pub(crate) fn render_shields(
    entries: &[CrapEntry],
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    write_badge(super::crappy_count(entries, threshold), threshold, out)
}

/// Delta-mode dispatch target. The badge ignores the baseline entirely and
/// counts the *current* scores carried inside the report, so the output is
/// identical to a run without `--baseline` (spec 15: no delta variant).
pub(crate) fn render_delta_shields(
    report: &DeltaReport,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let count = report
        .entries
        .iter()
        .filter(|e| Severity::classify(e.current.crap, threshold) == Severity::Crappy)
        .count();
    write_badge(count, threshold, out)
}

fn write_badge(
    count: usize,
    threshold: f64,
    out: &mut dyn Write,
) -> Result<()> {
    let (message, color) = match count {
        0 => ("passing".to_string(), "brightgreen"),
        1..=5 => (format!("{count} crappy"), "yellow"),
        _ => (format!("{count} crappy"), "red"),
    };
    let badge = Badge {
        schema_version: SCHEMA_VERSION,
        label: format!("CRAP > {}", format_threshold(threshold)),
        message,
        color,
    };
    serde_json::to_writer_pretty(&mut *out, &badge)?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_support::sample;
    use super::super::{Format, render, render_delta};
    use crate::delta::compute_delta;

    fn badge_value(count: usize) -> serde_json::Value {
        let mut buf = Vec::new();
        super::write_badge(count, 30.0, &mut buf).unwrap();
        serde_json::from_slice(&buf).expect("output must be valid JSON")
    }

    #[test]
    fn zero_crappy_is_a_passing_brightgreen_badge() {
        let v = badge_value(0);
        assert_eq!(v["schemaVersion"].as_u64(), Some(1));
        assert_eq!(v["label"].as_str(), Some("CRAP > 30"));
        assert_eq!(v["message"].as_str(), Some("passing"));
        assert_eq!(v["color"].as_str(), Some("brightgreen"));
    }

    #[test]
    fn label_formats_integral_threshold_without_trailing_zeros() {
        assert_eq!(super::format_threshold(15.0), "15");
        assert_eq!(super::format_threshold(30.0), "30");
    }

    #[test]
    fn label_keeps_fractional_threshold() {
        assert_eq!(super::format_threshold(12.5), "12.5");
    }

    #[test]
    fn one_crappy_is_yellow() {
        let v = badge_value(1);
        assert_eq!(v["message"].as_str(), Some("1 crappy"));
        assert_eq!(v["color"].as_str(), Some("yellow"));
    }

    #[test]
    fn five_crappy_is_still_yellow() {
        let v = badge_value(5);
        assert_eq!(v["message"].as_str(), Some("5 crappy"));
        assert_eq!(v["color"].as_str(), Some("yellow"));
    }

    #[test]
    fn six_crappy_is_red() {
        let v = badge_value(6);
        assert_eq!(v["message"].as_str(), Some("6 crappy"));
        assert_eq!(v["color"].as_str(), Some("red"));
    }

    #[test]
    fn badge_has_exactly_the_four_schema_keys() {
        let v = badge_value(0);
        let obj = v.as_object().expect("top-level object");
        assert_eq!(obj.len(), 4, "endpoint schema has exactly four keys: {v}");
    }

    #[test]
    fn render_counts_entries_above_threshold() {
        // sample() has one entry above 30 (CRAP 110) and one below (CRAP 1).
        let mut buf = Vec::new();
        render(&sample(), 30.0, Format::Shields, None, &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).expect("valid JSON");
        assert_eq!(v["message"].as_str(), Some("1 crappy"));
        assert_eq!(v["color"].as_str(), Some("yellow"));
    }

    #[test]
    fn render_with_high_threshold_is_passing() {
        let mut buf = Vec::new();
        render(&sample(), 200.0, Format::Shields, None, &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).expect("valid JSON");
        assert_eq!(v["message"].as_str(), Some("passing"));
        assert_eq!(v["color"].as_str(), Some("brightgreen"));
    }

    #[test]
    fn threshold_boundary_is_strictly_greater_than() {
        // An entry exactly at the threshold is not crappy (spec: CRAP > threshold).
        let mut buf = Vec::new();
        render(&sample(), 110.0, Format::Shields, None, &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).expect("valid JSON");
        assert_eq!(v["message"].as_str(), Some("passing"));
    }

    #[test]
    fn delta_mode_ignores_the_baseline_and_matches_plain_render() {
        let entries = sample();
        // A baseline that would normally produce regressions/removals.
        let mut baseline = sample();
        baseline[1].crap = 5.0;
        baseline.push(crate::merge::CrapEntry {
            file: "gone.rs".into(),
            function: "removed_fn".into(),
            line: 1,
            cyclomatic: 1.0,
            coverage: Some(100.0),
            crap: 1.0,
            crate_name: None,
        });
        let report = compute_delta(&entries, &baseline, 0.01);

        let mut delta_buf = Vec::new();
        render_delta(&report, 30.0, Format::Shields, None, false, &mut delta_buf).unwrap();
        let mut plain_buf = Vec::new();
        render(&entries, 30.0, Format::Shields, None, &mut plain_buf).unwrap();

        assert_eq!(
            String::from_utf8(delta_buf).unwrap(),
            String::from_utf8(plain_buf).unwrap(),
            "badge with --baseline must be byte-identical to the badge without it"
        );
    }
}
