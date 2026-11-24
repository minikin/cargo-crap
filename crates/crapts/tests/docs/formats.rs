use crate::coverage;
use crate::markers::Stream;
use crate::pages::{self, Page, Sample};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const OUTPUT_FORMATS: &str = "docs/crapts/reference/output-formats.md";
pub const JSON: &str = "docs/crapts/reference/json.md";
pub const FORMATS: [&str; 7] = [
    "text",
    "json",
    "markdown",
    "pr-comment",
    "github",
    "sarif",
    "shields",
];
pub const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const SCHEMA_PREFIX: &str =
    "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/";
pub fn schema_files() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(pages::repo_root().join("schemas/crapts"))
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let json = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"));
            json.then(|| path.file_name()?.to_str().map(str::to_owned))?
        })
        .collect();
    files.sort();
    files
}

pub fn check_output_formats(page: &Page) -> Result<(), String> {
    let mut found = BTreeSet::new();
    for sample in pages::samples(page)? {
        let Some(format) = sample.marker.effective_format() else {
            continue;
        };
        let args = sample.marker.args();
        let compares = |a: &String| {
            ["--baseline", "--base"]
                .iter()
                .any(|f| a == f || a.starts_with(&format!("{f}=")))
        };
        if args.iter().any(compares) {
            return Err(format!(
                "{}: an absolute sample passes a comparison",
                sample.label()
            ));
        }
        found.insert(format);
    }
    let expected: BTreeSet<String> = FORMATS.iter().map(|f| (*f).to_owned()).collect();
    let missing: Vec<_> = expected.difference(&found).collect();
    let extra: Vec<_> = found.difference(&expected).collect();
    if missing.is_empty() && extra.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: the samples lack the formats {missing:?} and add {extra:?}",
            page.path
        ))
    }
}

pub fn schema(file: &str) -> Value {
    serde_json::from_str(&pages::read(&format!("schemas/crapts/{file}"))).unwrap()
}

pub fn schema_ids() -> BTreeSet<String> {
    schema_files()
        .iter()
        .filter_map(|f| schema(f)["$id"].as_str().map(str::to_owned))
        .collect()
}

pub fn cited_schemas(page: &Page) -> BTreeSet<String> {
    page.text
        .match_indices(SCHEMA_PREFIX)
        .filter_map(|(at, _)| {
            let rest = &page.text[at..];
            let end = rest.find(|c: char| c.is_whitespace() || "`\")>]".contains(c))?;
            Some(rest[..end].to_owned())
        })
        .collect()
}

pub fn check_cited(
    page: &Page,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    let cited = cited_schemas(page);
    if let Some(unknown) = cited.difference(ids).next() {
        return Err(format!(
            "{}: {unknown} is not the $id of a schema",
            page.path
        ));
    }
    if let Some(missing) = ids.difference(&cited).next() {
        return Err(format!("{}: the page does not cite {missing}", page.path));
    }
    Ok(())
}

pub fn check_shape(
    sample: &Sample,
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    let Some(format) = sample.marker.effective_format() else {
        return Ok(());
    };
    if sample.marker.stream() == Stream::Stderr
        || !["json", "sarif", "shields"].contains(&format.as_str())
    {
        return Ok(());
    }
    let value = coverage::sample_json(sample)?;
    let schema = value["$schema"].as_str().unwrap_or_default();
    let fits = match format.as_str() {
        "json" => ids.contains(schema),
        "sarif" => schema == SARIF_SCHEMA && value["version"] == "2.1.0",
        _ => {
            let keys: BTreeSet<&str> = value
                .as_object()
                .map_or_default(|o| o.keys().map(String::as_str).collect());
            keys == BTreeSet::from(["schemaVersion", "label", "message", "color"])
                && value["schemaVersion"] == 1
        },
    };
    if fits {
        Ok(())
    } else {
        Err(format!(
            "{}: the {format} sample does not carry its format's schema",
            sample.label()
        ))
    }
}

pub fn check_shapes(
    samples: &[Sample],
    ids: &BTreeSet<String>,
) -> Result<usize, String> {
    for sample in samples {
        check_shape(sample, ids)?;
    }
    Ok(samples.len())
}

pub fn field_state(
    schema: &Value,
    field: &str,
) -> &'static str {
    let required = schema["required"]
        .as_array()
        .is_some_and(|r| r.iter().any(|k| k == field));
    if required {
        "required"
    } else if schema["properties"].get(field).is_some() {
        "optional"
    } else {
        "absent"
    }
}

/// Checked-in schemas by `$id`, so an external `$ref` resolves offline.
pub fn schemas_by_id() -> BTreeMap<String, Value> {
    schema_files()
        .iter()
        .map(|f| schema(f))
        .filter_map(|s| Some((s["$id"].as_str()?.to_owned(), s)))
        .collect()
}

fn resolve<'a>(
    root: &'a Value,
    node: &'a Value,
    external: &'a BTreeMap<String, Value>,
) -> Option<(&'a Value, &'a Value)> {
    let Some(reference) = node["$ref"].as_str() else {
        return Some((root, node));
    };
    match reference.strip_prefix("#/definitions/") {
        Some(name) => root["definitions"].get(name).map(|n| (root, n)),
        None => external.get(reference).map(|s| (s, s)),
    }
}

pub fn admits_null(
    root: &Value,
    node: &Value,
    external: &BTreeMap<String, Value>,
) -> bool {
    let Some((root, node)) = resolve(root, node, external) else {
        return false;
    };
    let typed = match &node["type"] {
        Value::String(t) => t == "null",
        Value::Array(types) => types.iter().any(|t| t == "null"),
        _ => false,
    };
    let branches = ["oneOf", "anyOf"]
        .iter()
        .filter_map(|k| node[*k].as_array())
        .flatten()
        .any(|branch| admits_null(root, branch, external));
    typed || branches || node.get("const").is_some_and(Value::is_null)
}

pub struct FieldRow {
    pub line: usize,
    pub field: String,
    pub report: String,
    pub delta: String,
    pub nullable: String,
}

pub fn field_rows(page: &Page) -> Result<Vec<FieldRow>, String> {
    let section = pages::section(page, "Top-level fields")?;
    pages::table_rows(&section)
        .into_iter()
        .map(|(line, cells)| match cells.as_slice() {
            [field, report, delta, nullable, ..] => Ok(FieldRow {
                line,
                field: pages::unquote(field).to_owned(),
                report: report.clone(),
                delta: delta.clone(),
                nullable: nullable.clone(),
            }),
            _ => Err(format!("{}: a field row needs four cells", page.at(line))),
        })
        .collect()
}

pub fn check_fields(
    page: &Page,
    report: &Value,
    delta: &Value,
    external: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let properties: BTreeSet<String> = [report, delta]
        .iter()
        .filter_map(|s| s["properties"].as_object())
        .flat_map(|p| p.keys().cloned())
        .collect();
    let rows = field_rows(page)?;
    let mut seen = BTreeSet::new();
    for row in &rows {
        if !properties.contains(&row.field) {
            return Err(format!(
                "{}: {} is not a top-level property",
                page.at(row.line),
                row.field
            ));
        }
        if !seen.insert(row.field.clone()) {
            return Err(format!(
                "{}: {} has a second row",
                page.at(row.line),
                row.field
            ));
        }
        let nullable = [report, delta].iter().any(|schema| {
            schema["properties"]
                .get(&row.field)
                .is_some_and(|node| admits_null(schema, node, external))
        });
        let expected = (
            field_state(report, &row.field),
            field_state(delta, &row.field),
            if nullable { "yes" } else { "no" },
        );
        let stated = (
            row.report.as_str(),
            row.delta.as_str(),
            row.nullable.as_str(),
        );
        if stated != expected {
            return Err(format!(
                "{}: {} reads {stated:?}, the schemas say {expected:?}",
                page.at(row.line),
                row.field
            ));
        }
    }
    let missing: Vec<_> = properties.difference(&seen).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: the field table lacks {missing:?}", page.path))
    }
}

pub fn check_against_schema(sample: &Sample) -> Result<(), String> {
    let text = sample
        .block
        .replace("\"<version>\"", &format!("\"{}\"", crate::fixture::VERSION));
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", sample.label()))?;
    let id = value["$schema"].as_str().unwrap_or_default();
    let file = schema_files()
        .into_iter()
        .find(|f| schema(f)["$id"] == id)
        .ok_or_else(|| format!("{}: {id} is not a report schema", sample.label()))?;
    let validator = crate::common::compile(&format!("schemas/crapts/{file}"));
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: {}", sample.label(), errors.join("; ")))
    }
}
