//! The Cases table in `specs/44-golden-outputs.md` is the case list: the
//! harness may hold no case the spec does not name, and miss none it does.

use std::collections::BTreeMap;

pub const SPEC: &str = "specs/44-golden-outputs.md";

pub fn spec_rows(spec: &str) -> Vec<(String, i32)> {
    spec.lines()
        .skip_while(|line| !line.starts_with("### Defaults"))
        .skip(1)
        .take_while(|line| !line.starts_with('#'))
        .filter_map(row)
        .collect()
}

fn row(line: &str) -> Option<(String, i32)> {
    let cells: Vec<&str> = line.strip_prefix('|')?.split('|').map(str::trim).collect();
    let id = *cells.first()?;
    let is_id = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let exit = cells.get(5)?.parse().ok()?;
    is_id.then(|| (id.to_owned(), exit))
}

pub fn compare(
    spec: &[(String, i32)],
    harness: &[(&str, i32)],
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut rows = BTreeMap::new();
    for (id, exit) in spec {
        if rows.insert(id.as_str(), *exit).is_some() {
            problems.push(format!("{id}: listed twice in the spec"));
        }
    }
    let mut cases = BTreeMap::new();
    for (id, exit) in harness {
        if cases.insert(*id, *exit).is_some() {
            problems.push(format!("{id}: listed twice in the harness"));
        }
    }
    for (id, exit) in &rows {
        match cases.get(id) {
            None => problems.push(format!("{id}: in the spec, missing from the harness")),
            Some(case) if case != exit => {
                problems.push(format!(
                    "{id}: the spec says exit {exit}, the harness {case}"
                ));
            },
            Some(_) => {},
        }
    }
    for id in cases.keys().filter(|id| !rows.contains_key(*id)) {
        problems.push(format!("{id}: in the harness, missing from the spec"));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
### Defaults

| ID | Fixture | Arguments | Config | Env | Exit | Notes |
|---|---|---|---|---|---|---|
| p_human | P | | - | | 0 | |
| p_fail_above | P | `--fail-above` | - | | 1 | |

## Tasks

| t_not_a_case | x | | - | | 0 | |
";

    fn rows() -> Vec<(String, i32)> {
        spec_rows(TABLE)
    }

    #[test]
    fn the_rows_are_the_table_under_defaults_only() {
        assert_eq!(
            rows(),
            [("p_human".to_owned(), 0), ("p_fail_above".to_owned(), 1)]
        );
    }

    #[test]
    fn a_matching_harness_has_no_problems() {
        assert_eq!(
            compare(&rows(), &[("p_human", 0), ("p_fail_above", 1)]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_case_missing_from_the_harness_is_named() {
        assert_eq!(
            compare(&rows(), &[("p_human", 0)]),
            ["p_fail_above: in the spec, missing from the harness"]
        );
    }

    #[test]
    fn a_case_missing_from_the_spec_is_named() {
        let problems = compare(
            &rows(),
            &[("p_human", 0), ("p_fail_above", 1), ("p_extra", 0)],
        );
        assert_eq!(problems, ["p_extra: in the harness, missing from the spec"]);
    }

    #[test]
    fn a_different_exit_is_named() {
        let problems = compare(&rows(), &[("p_human", 0), ("p_fail_above", 0)]);
        assert_eq!(
            problems,
            ["p_fail_above: the spec says exit 1, the harness 0"]
        );
    }

    #[test]
    fn a_duplicate_id_is_named() {
        let mut twice = rows();
        twice.push(("p_human".to_owned(), 0));
        let problems = compare(
            &twice,
            &[("p_human", 0), ("p_human", 0), ("p_fail_above", 1)],
        );
        assert_eq!(
            problems,
            [
                "p_human: listed twice in the spec",
                "p_human: listed twice in the harness"
            ]
        );
    }
}
