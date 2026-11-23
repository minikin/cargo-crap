use super::{Point, Statement, StatementFiles};
use crate::InputError;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

struct Object<'a> {
    names: Vec<String>,
    values: HashMap<String, &'a RawValue>,
}

impl<'a> Object<'a> {
    fn member(
        &self,
        key: &str,
    ) -> Option<&'a RawValue> {
        self.values.get(key).copied()
    }
}

fn object<'a>(
    raw: Option<&'a RawValue>,
    invalid_shape: &str,
) -> Result<Object<'a>, String> {
    let Some(raw) = raw.filter(|raw| raw.get().starts_with('{')) else {
        return Err(invalid_shape.to_owned());
    };
    serde_json::from_str(raw.get()).map_err(|error| format!("invalid JSON: {error}"))
}

struct ObjectVisitor;

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = Object<'de>;

    fn expecting(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(
        self,
        mut map: M,
    ) -> Result<Self::Value, M::Error> {
        let mut names = Vec::new();
        let mut values = HashMap::new();
        while let Some((name, value)) = map.next_entry::<String, &'de RawValue>()? {
            if values.insert(name.clone(), value).is_none() {
                names.push(name);
            }
        }
        // ECMAScript enumerates array indices first. All other names retain their insertion order.
        names.sort_by_cached_key(|name| {
            name.parse::<u32>()
                .ok()
                .filter(|n| n.to_string() == *name)
                .unwrap_or(u32::MAX)
        });
        Ok(Object { names, values })
    }
}

impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ObjectVisitor)
    }
}

fn integer(
    raw: Option<&RawValue>,
    minimum: f64,
) -> Option<f64> {
    let value: f64 = raw?.get().parse().ok()?;
    (value >= minimum && value.fract() == 0.0).then(|| value.abs())
}

fn start_point(start: &Object<'_>) -> Option<Point> {
    Some(Point {
        line: integer(start.member("line"), 1.0)?,
        column: integer(start.member("column"), 0.0)?,
    })
}

fn end_point(
    end: &Object<'_>,
    start: Point,
) -> Option<Point<Option<f64>>> {
    let line = integer(end.member("line"), start.line)?;
    let raw = end.member("column")?;
    let column = if raw.get() == "null" {
        None
    } else {
        let minimum = if line > start.line { 0.0 } else { start.column };
        Some(integer(Some(raw), minimum)?)
    };
    Some(Point { line, column })
}

fn statement(
    raw: &RawValue,
    hits: Option<&RawValue>,
) -> Result<Statement, String> {
    let span = object(Some(raw), "invalid span")?;
    let start = object(span.member("start"), "invalid span")?;
    let end = object(span.member("end"), "invalid span")?;
    let start = start_point(&start).ok_or("invalid start")?;
    let end = end_point(&end, start).ok_or("invalid end")?;
    let hits = integer(hits, 0.0).ok_or("invalid hit count")?;
    Ok(Statement { start, end, hits })
}

fn statements(raw: &RawValue) -> Result<Vec<Statement>, String> {
    let invalid_entry = "not an Istanbul coverage-final.json";
    let entry = object(Some(raw), invalid_entry)?;
    let spans = object(entry.member("statementMap"), invalid_entry)?;
    let hits = object(entry.member("s"), "missing or invalid hit map s")?;
    if let Some(extra) = hits
        .names
        .iter()
        .find(|name| !spans.values.contains_key(*name))
    {
        return Err(format!("statement {extra} has no span"));
    }
    spans
        .names
        .iter()
        .map(|id| {
            statement(spans.values[id], hits.member(id))
                .map_err(|detail| format!("statement {id}: {detail}"))
        })
        .collect()
}

pub(super) fn read(
    text: &str,
    label: &Path,
) -> Result<StatementFiles, InputError> {
    let files: Object<'_> = serde_json::from_str(text)
        .map_err(|err| InputError::at(label, format!("invalid JSON: {err}")))?;
    files
        .names
        .into_iter()
        .map(|key| {
            let observations = statements(files.values[&key])
                .map_err(|err| InputError::at(label, format!("{key}: {err}")))?;
            Ok((key, observations))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"{"src/a.ts":{"statementMap":{"x":{"start":{"line":2,"column":3},"end":{"line":2,"column":4}}},"s":{"x":0}}}"#;

    fn parse(text: &str) -> Result<Vec<(String, Vec<Statement>)>, InputError> {
        read(text, Path::new("run.json"))
    }

    fn error(
        text: &str,
        detail: &str,
    ) {
        assert_eq!(
            parse(text).unwrap_err().to_string(),
            format!("run.json: src/a.ts: {detail}"),
            "{text}"
        );
    }

    #[test]
    fn invalid_unicode_keys_report_json_decoding_errors_at_any_map_depth() {
        for text in [
            r#"{"\ud800.ts":{"statementMap":{},"s":{}}}"#,
            r#"{"a.ts":{"statementMap":{"\ud800":{}},"s":{}}}"#,
            r#"{"a.ts":{"statementMap":{},"s":{"\udfff":1}}}"#,
            r#"{"a.ts":{"statementMap":{"x":{"start":{"\ud800":0},"end":{}}},"s":{}}}"#,
        ] {
            let error = parse(text).unwrap_err();
            assert_eq!(error.path(), Path::new("run.json"));
            assert!(
                error.to_string().contains("invalid JSON"),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn validation_visits_files_and_complete_statements_in_member_order() {
        let span = r#"{"start":{"line":1,"column":0},"end":{"line":1,"column":null}}"#;
        let entry =
            format!(r#"{{"statementMap":{{"a":{span},"b":false}},"s":{{"a":null,"b":0}}}}"#);
        let valid = r#"{"statementMap":{},"s":{}}"#;
        for text in [
            format!(r#"{{"ok":{valid},"src/a.ts":{entry}}}"#),
            format!(r#"{{"src/a.ts":{entry},"ok":{valid}}}"#),
        ] {
            error(&text, "statement a: invalid hit count");
        }
        error(
            &format!(r#"{{"src/a.ts":{valid},"src/a.ts":false}}"#),
            "not an Istanbul coverage-final.json",
        );
        error(
            &format!(r#"{{"src/a.ts":{{"statementMap":{{"a":{span},"a":null}},"s":{{"a":0}}}}}}"#),
            "statement a: invalid span",
        );
    }

    #[test]
    fn map_ids_pair_each_span_with_its_hits() {
        let text = include_str!("../../tests/fixtures/coverage/statements.json");
        let files = parse(text).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "./src/a.ts");
        let observations: Vec<_> = files[0]
            .1
            .iter()
            .map(|s| {
                (
                    s.start.line,
                    s.start.column,
                    s.end.line,
                    s.end.column,
                    s.hits,
                )
            })
            .collect();
        assert_eq!(
            observations,
            [
                (1.0, 3.0, 1.0, Some(3.0), 0.0),
                (1.0, 3.0, 1.0, None, 1.0),
                (2.0, 9.0, 3.0, Some(0.0), 7.0)
            ]
        );
        assert_eq!(files[1], ("empty.ts".to_owned(), Vec::new()));
    }

    #[test]
    fn invalid_objects_fail_in_the_specified_order() {
        assert!(
            parse("[]")
                .unwrap_err()
                .to_string()
                .contains("expected a JSON object")
        );
        for entry in [
            "null",
            "[]",
            "{}",
            r#"{"total":3}"#,
            r#"{"statementMap":[]}"#,
        ] {
            error(
                &format!(r#"{{"src/a.ts":{entry}}}"#),
                "not an Istanbul coverage-final.json",
            );
        }
        for entry in [
            r#"{"statementMap":{}}"#,
            r#"{"statementMap":{},"s":null}"#,
            r#"{"statementMap":{},"s":[]}"#,
        ] {
            error(
                &format!(r#"{{"src/a.ts":{entry}}}"#),
                "missing or invalid hit map s",
            );
        }
        for invalid in ["null", "[]"] {
            for field in [
                r#"{"start":{"line":2,"column":3},"end":{"line":2,"column":4}}"#,
                r#"{"line":2,"column":3}"#,
                r#"{"line":2,"column":4}"#,
            ] {
                error(&BASE.replace(field, invalid), "statement x: invalid span");
            }
        }
        error(
            r#"{"src/a.ts":{"statementMap":{"x":false},"s":{"z":0,"b":0,"2":1,"1":0}}}"#,
            "statement 1 has no span",
        );
        error(
            r#"{"src/a.ts":{"statementMap":{"x":false},"s":{}}}"#,
            "statement x: invalid span",
        );
        error(
            r#"{"src/a.ts":{"statementMap":{"x":{"start":{},"end":null}},"s":{}}}"#,
            "statement x: invalid span",
        );
        error(
            r#"{"src/a.ts":{"statementMap":{"x":{"start":{},"end":{}}},"s":{}}}"#,
            "statement x: invalid start",
        );
        error(
            r#"{"src/a.ts":{"statementMap":{"x":{"start":{"line":1,"column":0},"end":{}}},"s":{}}}"#,
            "statement x: invalid end",
        );
        error(
            &BASE.replace(r#""x":0"#, ""),
            "statement x: invalid hit count",
        );
        for token in ["-1", "0.5", "null", "true", r#""1""#, "1e400"] {
            error(
                &BASE.replace(r#""x":0"#, &format!(r#""x":{token}"#)),
                "statement x: invalid hit count",
            );
        }
    }

    #[test]
    fn endpoint_validation_keeps_wide_coordinates_and_requires_explicit_end_columns() {
        for (field, values, detail) in [
            (
                r#"{"line":2,"column":3}"#,
                &[
                    r#"{"line":0,"column":3}"#,
                    r#"{"line":1.5,"column":3}"#,
                    r#"{"line":2,"column":-1}"#,
                    r#"{"line":2,"column":null}"#,
                    r#"{"line":2,"column":0.5}"#,
                    r#"{"line":1e400,"column":0}"#,
                ][..],
                "statement x: invalid start",
            ),
            (
                r#"{"line":2,"column":4}"#,
                &[
                    r#"{"line":1,"column":null}"#,
                    r#"{"line":2.5,"column":4}"#,
                    r#"{"line":2,"column":2}"#,
                    r#"{"line":3,"column":-1}"#,
                    r#"{"line":3,"column":0.5}"#,
                    r#"{"line":3}"#,
                ],
                "statement x: invalid end",
            ),
        ] {
            for value in values {
                error(&BASE.replace(field, value), detail);
            }
        }
        let wide = include_str!("../../tests/fixtures/coverage/wide.json");
        let statement = parse(wide).unwrap()[0].1[0];
        assert_eq!(statement.start.line, 9_007_199_254_740_992.0);
        assert_eq!(statement.start.column.to_bits(), 0);
        assert_eq!(statement.end.line, 9_007_199_254_740_994.0);
        assert_eq!(statement.hits, 1e300);
        let zero = parse(&wide.replace("1e300", "-0")).unwrap()[0].1[0];
        assert_eq!(zero.hits.to_bits(), 0);
        error(
            &wide.replace("9007199254740993", "-0"),
            "statement x: invalid start",
        );
        assert_eq!(
            parse(
                &BASE
                    .replace(r#""x":0"#, r#""x":1e0"#)
                    .replace(r#""column":3"#, r#""column":3.0"#)
            )
            .unwrap()[0]
                .1[0]
                .hits,
            1.0
        );
    }

    #[test]
    fn duplicate_values_replace_at_first_position_and_indices_sort_numerically() {
        let text = r#"{"x.ts":false,"./x.ts":{"statementMap":{},"s":{}},"x.ts":{
          "statementMap":{"b":null,"2":{"start":{"line":2,"column":0},"end":{"line":2,"column":1}},
                          "b":{"start":{"line":3,"column":0},"end":{"line":3,"column":null}}},
          "s":{"b":-1,"2":1,"b":4}}}"#;
        let files = parse(text).unwrap();
        assert_eq!(
            files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            ["x.ts", "./x.ts"]
        );
        let rows: Vec<_> = files[0].1.iter().map(|s| (s.start.line, s.hits)).collect();
        assert_eq!(rows, [(2.0, 1.0), (3.0, 4.0)]);

        let keys = [
            "b",
            "4294967295",
            "01",
            "4294967294",
            "10",
            "+2",
            "2",
            "0",
            "-0",
            "4294967296",
            "a",
        ];
        let members: Vec<_> = keys
            .iter()
            .map(|k| format!(r#""{k}":{{"statementMap":{{}},"s":{{}}}}"#))
            .collect();
        let files = parse(&format!("{{{}}}", members.join(","))).unwrap();
        assert_eq!(
            files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            [
                "0",
                "2",
                "10",
                "4294967294",
                "b",
                "4294967295",
                "01",
                "+2",
                "-0",
                "4294967296",
                "a"
            ]
        );
        for invalid in [
            r#"{"src/a.ts":false, "later": [}"#,
            r#"{"x":00,"x":{}}"#,
            r#"{"src/a.ts":false} other"#,
        ] {
            assert!(
                parse(invalid)
                    .unwrap_err()
                    .to_string()
                    .starts_with("run.json: invalid JSON"),
                "{invalid}"
            );
        }
    }
}
