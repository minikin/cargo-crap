use crate::markers::{Marker, Stream};
use crate::pages::{self, Page, Sample};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const CLI: &str = "docs/crapts/reference/cli.md";
pub const EXIT_CODES: &str = "docs/crapts/reference/exit-codes.md";

pub struct HelpOption {
    pub long: String,
    pub default: Option<String>,
}

pub fn all_help_options(help: &str) -> Vec<HelpOption> {
    let mut options: Vec<HelpOption> = Vec::new();
    for line in help.lines() {
        let indent = line.len() - line.trim_start().len();
        if indent <= 6 && line.trim_start().starts_with('-') {
            let Some(long) = line.split_whitespace().find(|w| w.starts_with("--")) else {
                continue;
            };
            options.push(HelpOption {
                long: long.trim_end_matches(',').to_owned(),
                default: None,
            });
        } else if let (Some(option), Some((_, rest))) =
            (options.last_mut(), line.split_once("[default: "))
        {
            option.default = Some(rest.trim_end().trim_end_matches(']').to_owned());
        }
    }
    options
}

pub fn help_options(help: &str) -> Vec<HelpOption> {
    let mut options = all_help_options(help);
    options.retain(|o| o.long != "--help" && o.long != "--version");
    options
}

pub fn flags_table(page: &Page) -> Result<Vec<(String, String, usize)>, String> {
    let section = pages::section(page, "Flags")?;
    let mut rows = Vec::new();
    for (line, cells) in pages::table_rows(&section) {
        let flag = pages::unquote(&cells[0]);
        let long = flag.split_whitespace().next().unwrap_or_default();
        if !long.starts_with("--") {
            return Err(format!("{}: `{flag}` is not a long flag", page.at(line)));
        }
        let default = cells.get(1).map(|c| pages::unquote(c).to_owned());
        rows.push((long.to_owned(), default.unwrap_or_default(), line));
    }
    Ok(rows)
}

pub fn check_flag_rows(
    page: &Page,
    help: &str,
) -> Result<(), String> {
    let heading = pages::section(page, "Flags")?.first_line;
    let mut listed = BTreeSet::new();
    for (long, _, line) in flags_table(page)? {
        if !listed.insert(long.clone()) {
            return Err(format!("{}: {long} has a second row", page.at(line)));
        }
    }
    let printed: BTreeSet<String> = help_options(help).into_iter().map(|o| o.long).collect();
    let missing: Vec<_> = printed.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&printed).collect();
    if missing.is_empty() && extra.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: the Flags table lacks {missing:?} and lists {extra:?}, which --help does not print",
            page.at(heading)
        ))
    }
}

pub fn default_cell(value: &Value) -> String {
    match value {
        Value::Null => "none".to_owned(),
        Value::Array(items) if items.is_empty() => "none".to_owned(),
        Value::Bool(false) => "off".to_owned(),
        Value::Bool(true) => "on".to_owned(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn expected_defaults(
    help: &str,
    print_config: &Value,
) -> BTreeMap<String, String> {
    help_options(help)
        .into_iter()
        .map(|option| {
            let setting = option.long.trim_start_matches("--").replace('-', "_");
            let expected = option.default.clone().unwrap_or_else(|| {
                let settings = &print_config["settings"];
                let leaf = match setting.as_str() {
                    "duplicates" => Some(&settings["duplicates"]["enabled"]),
                    "dup_threshold" => Some(&settings["duplicates"]["min_similarity"]),
                    key => settings.get(key),
                };
                leaf.map_or_else(|| "off".to_owned(), |s| default_cell(&s["value"]))
            });
            (option.long, expected)
        })
        .collect()
}

pub fn check_defaults(
    page: &Page,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (long, stated, line) in flags_table(page)? {
        let Some(want) = expected.get(&long) else {
            continue;
        };
        if &stated != want {
            return Err(format!(
                "{}: the Default of {long} reads {stated}, the binary uses {want}",
                page.at(line)
            ));
        }
    }
    Ok(())
}

/// Only the crapts column is checked: this test binary cannot run cargo-crap.
pub fn check_differences(
    page: &Page,
    help: &str,
) -> Result<usize, String> {
    let section = pages::section(page, "Differences from cargo-crap")?;
    let printed: BTreeSet<String> = all_help_options(help).into_iter().map(|o| o.long).collect();
    let mut named = 0;
    for (line, cells) in pages::table_rows(&section) {
        let crapts = cells.get(1).map_or_default(String::as_str);
        for flag in long_flags(crapts) {
            if !printed.contains(&flag) {
                return Err(format!("{}: {flag} is not a crapts option", page.at(line)));
            }
            named += 1;
        }
    }
    Ok(named)
}

pub fn long_flags(text: &str) -> Vec<String> {
    text.match_indices("--")
        .filter(|(at, _)| {
            text[..*at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-')
        })
        .filter_map(|(at, _)| {
            let name: String = text[at + 2..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            name.starts_with(|c: char| c.is_ascii_alphanumeric())
                .then(|| format!("--{name}"))
        })
        .collect()
}

pub fn exit_codes(page: &Page) -> Result<Vec<(u8, usize)>, String> {
    let mut codes = Vec::new();
    for (line, cells) in pages::table_rows(page) {
        let cell = pages::unquote(&cells[0]);
        let code = cell
            .parse()
            .map_err(|_| format!("{}: `{cell}` is not an exit code", page.at(line)))?;
        codes.push((code, line));
    }
    Ok(codes)
}

pub fn check_exit_codes(
    page: &Page,
    samples: &[Sample],
) -> Result<(), String> {
    let codes = exit_codes(page)?;
    for (code, line) in &codes {
        if *code > 2 {
            return Err(format!(
                "{}: exit code {code} is not one crapts uses",
                page.at(*line)
            ));
        }
        if !samples.iter().any(|s| s.marker.exit() == *code) {
            return Err(format!(
                "{}: no marker demonstrates exit {code}",
                page.at(*line)
            ));
        }
    }
    let listed: BTreeSet<u8> = codes.iter().map(|(c, _)| *c).collect();
    if listed != BTreeSet::from([0, 1, 2]) {
        return Err(format!(
            "{}: the table lists codes {listed:?}, expected 0, 1 and 2",
            page.path
        ));
    }
    let stderr_2 = samples.iter().any(|s| {
        matches!(
            s.marker,
            Marker::Run {
                stream: Stream::Stderr,
                exit: 2,
                ..
            }
        )
    });
    if stderr_2 {
        Ok(())
    } else {
        Err(format!(
            "{}: no crapts-stderr marker demonstrates exit 2",
            page.path
        ))
    }
}
