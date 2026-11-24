use crate::markers;
use crate::pages::{self, Page, Sample};
use serde_json::Value;

pub const COVERAGE: &str = "docs/crapts/coverage.md";
pub const PRODUCERS: [&str; 4] = ["Vitest", "Jest", "Istanbul", "LCOV"];

pub fn coverage_argument(args: &[String]) -> Option<String> {
    markers::option_value(args, "--coverage").map(str::to_owned)
}

fn stated_file(section: &Page) -> Option<(usize, String)> {
    section
        .lines()
        .iter()
        .filter(|l| !l.fenced && !l.fence)
        .find_map(|l| {
            let (_, after) = l.text.split_once("writes `")?;
            after.split('`').next().map(|f| (l.number, f.to_owned()))
        })
}

pub fn check_section(
    page: &Page,
    producer: &str,
) -> Result<(), String> {
    let section = pages::section(page, producer)?;
    let heading = page.at(section.first_line);
    let mut producer_line = None;
    let mut reads = None;
    for (number, line) in pages::command_lines(&section) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        match pages::crapts_arguments(&line).map_err(|e| format!("{}: {e}", page.at(number)))? {
            Some(args) => reads = reads.or_else(|| Some((number, coverage_argument(&args)?))),
            None => producer_line = producer_line.or(Some(line)),
        }
    }
    if producer_line.is_none() {
        return Err(format!("{heading}: {producer} has no producer command"));
    }
    let (written_at, writes) = stated_file(&section)
        .ok_or_else(|| format!("{heading}: {producer} does not state the file it writes"))?;
    let (read_at, reads) =
        reads.ok_or_else(|| format!("{heading}: {producer} has no crapts --coverage command"))?;
    if reads == writes {
        Ok(())
    } else {
        Err(format!(
            "{}: {producer} writes {writes} (line {written_at}) but its crapts command reads {reads}",
            page.at(read_at)
        ))
    }
}

pub fn check_sections(page: &Page) -> Result<(), String> {
    for producer in PRODUCERS {
        check_section(page, producer)?;
    }
    Ok(())
}

pub fn sample_json(sample: &Sample) -> Result<Value, String> {
    serde_json::from_str(&sample.block).map_err(|e| format!("{}: {e}", sample.label()))
}

pub fn crap_of(
    report: &Value,
    function: &str,
) -> Option<f64> {
    report["entries"]
        .as_array()?
        .iter()
        .find(|e| e["function"] == function)?["crap"]
        .as_f64()
}

pub fn kind_sample(
    page: &Page,
    file: &str,
) -> Result<(String, Value), String> {
    let samples = pages::samples(page)?;
    let sample = samples
        .iter()
        .find(|s| {
            s.marker.effective_format().as_deref() == Some("json")
                && coverage_argument(&s.marker.args()).as_deref() == Some(file)
        })
        .ok_or_else(|| format!("{}: no marked JSON sample reads {file}", page.path))?;
    let report = sample_json(sample)?;
    let kind = report["coverage_kind"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    Ok((kind, report))
}

pub fn text_rows(block: &str) -> Vec<(String, String, String)> {
    pages::box_rows(block)
        .into_iter()
        .filter_map(|cells| match cells.as_slice() {
            [_, _, cc, coverage, function, _] if cc.parse::<u32>().is_ok() => {
                Some((function.clone(), cc.clone(), coverage.clone()))
            },
            _ => None,
        })
        .collect()
}
