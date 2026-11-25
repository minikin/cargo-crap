use crate::fixture::Repo;
use crate::pages::{self, Page};
use serde_json::Value;
use std::collections::BTreeSet;

pub const CONFIG: &str = "docs/crapts/reference/config.md";

pub const CONFIG_KEYS: [&str; 28] = [
    "threshold",
    "fail-above",
    "missing",
    "exclude",
    "default-excludes",
    "allow",
    "min",
    "top",
    "sort",
    "epsilon",
    "fail-regression",
    "show-unchanged",
    "fail-changed",
    "uncovered-hints",
    "signals.enabled",
    "signals.gate",
    "signals.limits.nesting",
    "signals.limits.lines",
    "signals.limits.bumps",
    "signals.limits.bool-ops",
    "duplicates.enabled",
    "duplicates.min-similarity",
    "duplicates.min-nodes",
    "duplicates.triage.enabled",
    "duplicates.triage.provider",
    "duplicates.triage.model",
    "duplicates.triage.confidence-floor",
    "duplicates.triage.max-requests",
];

pub const PER_RUN: [&str; 11] = [
    "paths",
    "coverage",
    "format",
    "summary",
    "all-rows",
    "output",
    "repo-url",
    "commit-ref",
    "comment-marker",
    "baseline",
    "base",
];

pub fn example(page: &Page) -> Result<(usize, String, toml::Table), String> {
    let block = page
        .blocks()
        .into_iter()
        .find(|b| b.info.split_whitespace().next() == Some("toml"))
        .ok_or_else(|| format!("{}: no toml block", page.path))?;
    let table = block
        .body
        .parse()
        .map_err(|e| format!("{}: the example is not TOML: {e}", page.at(block.open_line)))?;
    Ok((block.open_line, block.body, table))
}

pub fn print_config(
    repo: &Repo,
    args: &[String],
) -> Result<Value, String> {
    let mut full = vec!["--print-config".to_owned()];
    full.extend(args.iter().cloned());
    let run = repo.crapts(&full);
    if run.code != Some(0) {
        return Err(format!(
            "crapts {full:?} exited {:?}: {}",
            run.code,
            run.stderr.trim()
        ));
    }
    serde_json::from_str(&run.stdout).map_err(|e| format!("crapts {full:?}: {e}"))
}

fn setting<'a>(
    config: &'a Value,
    key: &str,
) -> &'a Value {
    key.split('.').fold(&config["settings"], |table, part| {
        &table[part.replace('-', "_")]
    })
}

pub fn origin(
    config: &Value,
    key: &str,
) -> String {
    setting(config, key)["origin"]
        .as_str()
        .unwrap_or("absent")
        .to_owned()
}

fn leaves(
    table: &toml::Table,
    prefix: &str,
    keys: &mut BTreeSet<String>,
) {
    for (key, value) in table {
        let name = format!("{prefix}{}", key.replace('_', "-"));
        if let Some(children) = value.as_table() {
            leaves(children, &format!("{name}."), keys);
        } else {
            keys.insert(name);
        }
    }
}

pub fn check_example(
    page: &Page,
    repo: &Repo,
) -> Result<(), String> {
    let (line, body, table) = example(page)?;
    let mut set = BTreeSet::new();
    leaves(&table, "", &mut set);
    let expected: BTreeSet<String> = CONFIG_KEYS.iter().map(|k| (*k).to_owned()).collect();
    let missing: Vec<_> = expected.difference(&set).collect();
    let extra: Vec<_> = set.difference(&expected).collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(format!(
            "{}: the example lacks {missing:?} and sets {extra:?}, which is not a config key",
            page.at(line)
        ));
    }
    let config = with_config(repo, &body)?;
    for key in &set {
        if key.starts_with("duplicates.") {
            let expected =
                serde_json::to_value(example_value(&table, key)).map_err(|e| e.to_string())?;
            let actual = &setting(&config, key)["value"];
            if actual != &expected {
                return Err(format!(
                    "{}: {key} prints {actual}, expected {expected}",
                    page.at(line)
                ));
            }
        }
        let origin = origin(&config, key);
        if origin != "config" {
            return Err(format!(
                "{}: {key} has origin {origin}, expected config",
                page.at(line)
            ));
        }
    }
    Ok(())
}

pub struct KeyRow {
    pub line: usize,
    pub key: String,
    pub aliases: Vec<String>,
    pub flag: Option<String>,
    pub kind: String,
    pub default: String,
}

const KINDS: [(&str, &[&str]); 28] = [
    ("threshold", &["finite number"]),
    ("fail-above", &["boolean"]),
    ("missing", &["`pessimistic`", "`optimistic`", "`skip`"]),
    ("exclude", &["list of globs"]),
    ("default-excludes", &["list of globs"]),
    ("allow", &["list of globs"]),
    ("min", &["finite number"]),
    ("top", &["integer", "1", "9007199254740991"]),
    ("sort", &["`crap`", "`file`"]),
    ("epsilon", &["finite number", "at least 0"]),
    ("fail-regression", &["boolean"]),
    ("show-unchanged", &["boolean"]),
    ("fail-changed", &["boolean"]),
    ("uncovered-hints", &["boolean"]),
    ("signals.enabled", &["boolean"]),
    ("signals.gate", &["list of signal names"]),
    ("signals.limits.nesting", &["integer", "0", "4294967295"]),
    ("signals.limits.lines", &["integer", "0", "4294967295"]),
    ("signals.limits.bumps", &["integer", "0", "4294967295"]),
    ("signals.limits.bool-ops", &["integer", "0", "4294967295"]),
    ("duplicates.enabled", &["boolean"]),
    ("duplicates.min-similarity", &["finite number from 0 to 1"]),
    ("duplicates.min-nodes", &["nonnegative integer"]),
    ("duplicates.triage.enabled", &["boolean"]),
    ("duplicates.triage.provider", &["typesafe or openai"]),
    ("duplicates.triage.model", &["string"]),
    (
        "duplicates.triage.confidence-floor",
        &["finite number from 0 to 1"],
    ),
    (
        "duplicates.triage.max-requests",
        &["integer from 0 to 4294967295"],
    ),
];

pub fn key_rows(page: &Page) -> Result<Vec<KeyRow>, String> {
    let section = pages::section(page, "Keys")?;
    let mut rows = Vec::new();
    for (line, cells) in pages::table_rows(&section) {
        if cells.len() != 5 {
            return Err(format!(
                "{}: a key row needs key, alias, flag, type and default cells",
                page.at(line)
            ));
        }
        let aliases = cells[1]
            .split(", ")
            .map(pages::unquote)
            .filter(|name| *name != "none")
            .map(str::to_owned)
            .collect();
        let flag = pages::unquote(&cells[2]);
        rows.push(KeyRow {
            line,
            key: pages::unquote(&cells[0]).to_owned(),
            aliases,
            flag: (flag != "config only").then(|| flag.to_owned()),
            kind: cells[3].clone(),
            default: pages::unquote(&cells[4]).to_owned(),
        });
    }
    Ok(rows)
}

pub fn per_run_rows(page: &Page) -> Result<Vec<(usize, String)>, String> {
    let section = pages::section(page, "Per-run settings")?;
    Ok(pages::table_rows(&section)
        .into_iter()
        .map(|(line, cells)| (line, pages::unquote(&cells[0]).to_owned()))
        .collect())
}

pub fn check_per_run(page: &Page) -> Result<(), String> {
    let rows = per_run_rows(page)?;
    let keys: BTreeSet<String> = key_rows(page)?.into_iter().map(|r| r.key).collect();
    for (line, name) in &rows {
        if keys.contains(name) || !PER_RUN.contains(&name.as_str()) {
            return Err(format!(
                "{}: {name} is not a per-run setting",
                page.at(*line)
            ));
        }
    }
    let named: BTreeSet<&str> = rows.iter().map(|(_, n)| n.as_str()).collect();
    let missing: Vec<_> = PER_RUN.iter().filter(|s| !named.contains(**s)).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: the per-run table lacks {missing:?}",
            page.path
        ))
    }
}

pub fn flag_probe(
    key: &str,
    flag: &str,
    value: &toml::Value,
) -> Vec<String> {
    let mut args = Vec::new();
    match value {
        _ if flag.starts_with("--no-") => args.push(flag.to_owned()),
        toml::Value::Boolean(_) => args.push(flag.to_owned()),
        toml::Value::Array(items) => {
            for item in items {
                args.extend([flag.to_owned(), scalar(item)]);
            }
        },
        other => args.extend([flag.to_owned(), scalar(other)]),
    }
    if ["epsilon", "fail-regression", "show-unchanged"].contains(&key) {
        args.extend(["--baseline".to_owned(), "crapts-baseline.json".to_owned()]);
    }
    args
}

fn scalar(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

fn with_config(
    repo: &Repo,
    text: &str,
) -> Result<Value, String> {
    let file = repo.root().join(".crapts.toml");
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    let config = print_config(repo, &[]);
    std::fs::remove_file(file).map_err(|e| e.to_string())?;
    config
}

fn example_value<'a>(
    example: &'a toml::Table,
    key: &str,
) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let first = example.get(parts.next()?)?;
    parts.try_fold(first, |table, part| table.get(part))
}

pub fn check_keys(
    page: &Page,
    repo: &Repo,
) -> Result<(), String> {
    let (_, _, example) = example(page)?;
    let rows = key_rows(page)?;
    for row in &rows {
        for alias in &row.aliases {
            check_alias(page, repo, row, alias, &example)?;
        }
        let Some(flag) = &row.flag else {
            continue;
        };
        let value = example_value(&example, &row.key).ok_or_else(|| {
            format!(
                "{}: the example does not set {}",
                page.at(row.line),
                row.key
            )
        })?;
        let args = flag_probe(&row.key, flag, value);
        let config = print_config(repo, &args)?;
        let origin = origin(&config, &row.key);
        if origin != "flag" {
            return Err(format!(
                "{}: {} gives {} origin {origin}, expected flag",
                page.at(row.line),
                args.join(" "),
                row.key
            ));
        }
    }
    let config_only: Vec<&str> = rows
        .iter()
        .filter(|r| r.flag.is_none())
        .map(|r| r.key.as_str())
        .collect();
    let expected: Vec<_> = CONFIG_KEYS
        .iter()
        .copied()
        .filter(|key| {
            *key == "uncovered-hints"
                || key.starts_with("signals.")
                || (key.starts_with("duplicates.")
                    && !matches!(*key, "duplicates.enabled" | "duplicates.min-similarity"))
        })
        .collect();
    if config_only == expected {
        Ok(())
    } else {
        Err(format!(
            "{}: the keys marked config only are {config_only:?}, expected {expected:?}",
            page.path
        ))
    }
}

fn check_alias(
    page: &Page,
    repo: &Repo,
    row: &KeyRow,
    alias: &str,
    example: &toml::Table,
) -> Result<(), String> {
    let value = example_value(example, &row.key).ok_or_else(|| {
        format!(
            "{}: the example does not set {}",
            page.at(row.line),
            row.key
        )
    })?;
    // These fixture values use the same literal syntax in TOML and JSON.
    let literal = serde_json::to_string(value).map_err(|e| e.to_string())?;
    let aliased = format!("{alias} = {literal}\n");
    let config = with_config(repo, &aliased)
        .map_err(|e| format!("{}: alias {alias}: {e}", page.at(row.line)))?;
    match origin(&config, &row.key).as_str() {
        "config" => Ok(()),
        other => Err(format!(
            "{}: alias {alias} gives origin {other}",
            page.at(row.line)
        )),
    }
}

pub fn aliases(page: &Page) -> Result<BTreeSet<String>, String> {
    Ok(key_rows(page)?
        .into_iter()
        .flat_map(|r| r.aliases)
        .collect())
}

pub fn refusal(
    repo: &Repo,
    line: &str,
) -> crate::fixture::Run {
    let file = repo.root().join(".crapts.toml");
    std::fs::write(&file, format!("{line}\n")).unwrap();
    let run = repo.crapts(&["--print-config"]);
    std::fs::remove_file(file).unwrap();
    run
}

const COUNTS: [&str; 12] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
    "twelve",
];

pub fn default_cell(value: &Value) -> String {
    match value {
        Value::Null => "unset".to_owned(),
        Value::Array(items) if items.is_empty() => "empty".to_owned(),
        Value::Array(items) => format!(
            "the {} patterns below",
            COUNTS.get(items.len() - 1).copied().unwrap_or("many")
        ),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn check_types_and_defaults(
    page: &Page,
    defaults: &Value,
) -> Result<(), String> {
    for row in key_rows(page)? {
        let words = KINDS
            .iter()
            .find(|(key, _)| *key == row.key)
            .map(|(_, words)| *words)
            .ok_or_else(|| format!("{}: {} is not a config key", page.at(row.line), row.key))?;
        if let Some(word) = words.iter().find(|w| !row.kind.contains(**w)) {
            return Err(format!(
                "{}: the type of {} reads {}, which lacks {word}",
                page.at(row.line),
                row.key,
                row.kind
            ));
        }
        let value = &setting(defaults, &row.key)["value"];
        let expected = if row.key == "signals.gate" {
            value
                .as_array()
                .expect("the gate list")
                .iter()
                .map(|name| name.as_str().expect("a signal name"))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            default_cell(value)
        };
        if row.default != expected {
            return Err(format!(
                "{}: the default of {} reads {}, the binary uses {expected}",
                page.at(row.line),
                row.key,
                row.default
            ));
        }
    }
    Ok(())
}

pub fn listed_default_excludes(page: &Page) -> Result<Vec<String>, String> {
    let section = pages::section(page, "Keys")?;
    let block = section
        .blocks()
        .into_iter()
        .find(|b| b.info == "text")
        .ok_or_else(|| format!("{}: no list of default excludes", page.path))?;
    Ok(block.body.lines().map(str::to_owned).collect())
}
