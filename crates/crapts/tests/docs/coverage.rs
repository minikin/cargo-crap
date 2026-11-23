use crate::pages::{self, Page, Sample};
use serde_json::Value;

pub const COVERAGE: &str = "docs/crapts/coverage.md";
pub const PRODUCERS: [&str; 4] = ["Vitest", "Jest", "Istanbul", "LCOV"];

pub fn coverage_argument(args: &[String]) -> Option<String> {
    args.iter()
        .enumerate()
        .find_map(|(i, a)| match a.strip_prefix("--coverage") {
            Some("") => args.get(i + 1).cloned(),
            Some(rest) => rest.strip_prefix('=').map(str::to_owned),
            None => None,
        })
}

/// The file a section says its producer writes: the inline code after
/// the word "writes" in its prose.
fn stated_file(section: &Page) -> Option<String> {
    section
        .lines()
        .iter()
        .filter(|l| !l.fenced && !l.fence)
        .find_map(|l| {
            let (_, after) = l.text.split_once("writes `")?;
            after.split('`').next().map(str::to_owned)
        })
}

pub fn check_section(
    page: &Page,
    producer: &str,
) -> Result<(), String> {
    let section = pages::section(page, producer)?;
    let mut producer_line = None;
    let mut reads = None;
    for (_, line) in pages::command_lines(&section) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        match pages::crapts_arguments(&line)? {
            Some(args) => reads = reads.or_else(|| coverage_argument(&args)),
            None => producer_line = producer_line.or(Some(line)),
        }
    }
    if producer_line.is_none() {
        return Err(format!("{}: {producer} has no producer command", page.path));
    }
    let writes = stated_file(&section).ok_or_else(|| {
        format!(
            "{}: {producer} does not state the file it writes",
            page.path
        )
    })?;
    let reads = reads
        .ok_or_else(|| format!("{}: {producer} has no crapts --coverage command", page.path))?;
    if reads == writes {
        Ok(())
    } else {
        Err(format!(
            "{}: {producer} writes {writes} but its crapts command reads {reads}",
            page.path
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
    block
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line
                .split(['\u{2502}', '\u{2506}'])
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            match cells.as_slice() {
                [_, _, cc, coverage, function, _] if cc.parse::<u32>().is_ok() => Some((
                    (*function).to_owned(),
                    (*cc).to_owned(),
                    (*coverage).to_owned(),
                )),
                _ => None,
            }
        })
        .collect()
}
