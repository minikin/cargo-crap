use crate::coverage;
use crate::markers;
use crate::pages::{self, Page, Sample};
use serde_json::Value;

pub const BASELINE: &str = "docs/crapts/guides/baseline.md";
pub const CHANGED_GATE: &str = "docs/crapts/guides/changed-gate.md";

pub const REGRESSION_ARGS: [&str; 6] = [
    "--coverage",
    "coverage/lcov.info",
    "--baseline",
    "crapts-baseline.json",
    "--fail-regression",
    "--show-unchanged",
];
pub const CHANGED_ARGS: [&str; 7] = [
    "--coverage",
    "coverage/lcov.info",
    "--base",
    "main",
    "--fail-changed",
    "--format",
    "json",
];
pub const COMBINED_ARGS: [&str; 10] = [
    "--coverage",
    "coverage/lcov.info",
    "--base",
    "main",
    "--baseline",
    "crapts-baseline.json",
    "--fail-changed",
    "--fail-regression",
    "--format",
    "json",
];

pub fn find_sample<'a>(
    page: &Page,
    samples: &'a [Sample],
    args: &[&str],
    exit: u8,
) -> Result<&'a Sample, String> {
    samples
        .iter()
        .find(|s| s.marker.args() == args && s.marker.exit() == exit)
        .ok_or_else(|| {
            format!(
                "{}: no marker runs `{}` with expected exit {exit}",
                page.path,
                args.join(" ")
            )
        })
}

pub fn check_regression_sample(sample: &Sample) -> Result<(), String> {
    let rows = pages::box_rows(&sample.block);
    let header = rows
        .first()
        .ok_or_else(|| format!("{}: no table", sample.label()))?;
    let column = |name: &str| header.iter().position(|c| c == name);
    let (Some(crap), Some(delta), Some(function)) =
        (column("CRAP"), column("\u{394}"), column("Function"))
    else {
        return Err(format!(
            "{}: the table lacks CRAP, delta or Function",
            sample.label()
        ));
    };
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.get(function).map(String::as_str) == Some(name))
    };
    let checkout = row("checkout").map(|r| (r[crap].as_str(), r[delta].as_str()));
    let total = row("total").map(|r| (r[crap].as_str(), r[delta].as_str()));
    if checkout != Some(("42.0", "+30.0")) || total != Some(("1.0", "")) {
        return Err(format!(
            "{}: expected checkout 42.0 regressed by +30.0 from 12 and total unchanged, got {checkout:?} and {total:?}",
            sample.label()
        ));
    }
    let footer = sample.block.lines().last().unwrap_or_default();
    if footer.contains("1 regressed") && footer.contains("1 unchanged") {
        Ok(())
    } else {
        Err(format!(
            "{}: the counts line reads `{footer}`",
            sample.label()
        ))
    }
}

pub fn check_baseline_writer(page: &Page) -> Result<usize, String> {
    let mut writers = 0;
    for (line, text) in pages::command_lines(page) {
        let Some(args) =
            pages::crapts_arguments(&text).map_err(|e| format!("{}: {e}", page.at(line)))?
        else {
            continue;
        };
        if markers::option_value(&args, "--output").is_none() {
            continue;
        }
        let json = markers::option_value(&args, "--format") == Some("json");
        if !json || !args.iter().any(|a| a == "--all-rows") {
            return Err(format!(
                "{}: `{text}` writes a baseline without --format json and --all-rows",
                page.at(line)
            ));
        }
        writers += 1;
    }
    if writers == 0 {
        return Err(format!("{}: no command writes the baseline", page.path));
    }
    Ok(writers)
}

pub fn check_changed_entry(sample: &Sample) -> Result<Value, String> {
    let report = coverage::sample_json(sample)?;
    let entries = report["changed"]["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let [entry] = entries else {
        return Err(format!(
            "{}: the changed section has {} entries, expected checkout alone",
            sample.label(),
            entries.len()
        ));
    };
    let facts = (
        entry["function"].as_str(),
        entry["change"].as_str(),
        entry["base_cyclomatic"].as_u64(),
        entry["cyclomatic"].as_u64(),
        entry["crap"].as_f64(),
        entry["verdict"].as_str(),
    );
    let expected = (
        Some("checkout"),
        Some("modified"),
        Some(3),
        Some(6),
        Some(42.0),
        Some("fail"),
    );
    if facts == expected {
        Ok(report)
    } else {
        Err(format!(
            "{}: the changed entry is {facts:?}, expected {expected:?}",
            sample.label()
        ))
    }
}

pub fn check_combined(sample: &Sample) -> Result<(), String> {
    let report = check_changed_entry(sample)?;
    let checkout = report["entries"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["function"] == "checkout"))
        .ok_or_else(|| format!("{}: no checkout row", sample.label()))?;
    let facts = (
        checkout["status"].as_str(),
        checkout["baseline_crap"].as_f64(),
        checkout["crap"].as_f64(),
    );
    if facts == (Some("regressed"), Some(12.0), Some(42.0)) {
        Ok(())
    } else {
        Err(format!(
            "{}: checkout is {facts:?} against the baseline, expected regressed from 12 to 42",
            sample.label()
        ))
    }
}
