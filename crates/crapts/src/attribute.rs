use crate::coverage::{Coverage, CoverageData, CoverageUnit, Point, Statement};
use crate::{FileAnalysis, Position, Row, SourceSpan};
use cargo_crap_core::coverage::{FileCoverage, StrictLcovFile};
use cargo_crap_core::entry::{CrapEntry, MissingCoveragePolicy, ScopeDiagnostics, StrayFiles};
use cargo_crap_core::paths::{self, Assignment, TiePolicy};
use cargo_crap_core::score::crap;
use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

#[derive(Debug)]
pub struct AttributedRow {
    pub entry: CrapEntry,
    pub covered: usize,
    pub total: usize,
}

#[derive(Debug)]
pub struct Attribution {
    pub rows: Vec<AttributedRow>,
    pub unit: CoverageUnit,
    pub diagnostics: ScopeDiagnostics,
    pub warnings: Vec<String>,
}

struct FileRows<'a> {
    name: Cow<'a, str>,
    rows: Vec<&'a Row>,
}

fn group_rows(analyses: &[FileAnalysis]) -> Vec<FileRows<'_>> {
    let mut files = Vec::new();
    let mut indices = HashMap::new();
    for row in analyses.iter().flat_map(|a| &a.rows) {
        let name = row.file.to_string_lossy();
        let i = *indices.entry(name.clone()).or_insert_with(|| {
            let i = files.len();
            files.push(FileRows {
                name,
                rows: Vec::new(),
            });
            i
        });
        files[i].rows.push(row);
    }
    files
}

fn diagnostics(
    assignment: &Assignment,
    sources: &[&str],
    keys: &[&str],
) -> ScopeDiagnostics {
    let used: HashSet<_> = assignment.bindings.iter().flatten().copied().collect();
    let source_only: Vec<_> = sources
        .iter()
        .zip(&assignment.bindings)
        .filter(|(_, group)| group.is_empty())
        .map(|(&source, _)| source)
        .collect();
    let coverage_only: Vec<_> = keys
        .iter()
        .enumerate()
        .filter(|(k, _)| !used.contains(k))
        .map(|(_, &key)| key)
        .collect();
    ScopeDiagnostics {
        analyzed_files: sources.len(),
        lcov_files: keys.len(),
        matched_files: sources.len() - source_only.len(),
        source_only: StrayFiles::from_raw_spellings(&source_only),
        lcov_only: StrayFiles::from_raw_spellings(&coverage_only),
    }
}

fn overlap_warning(d: &ScopeDiagnostics) -> Option<String> {
    if d.analyzed_files == 0 || d.lcov_files == 0 {
        return None;
    }
    let matched = d.matched_files;
    if matched == 0 {
        Some("no coverage file overlaps the analyzed files: check that the coverage report was produced for this source tree".to_owned())
    } else if matched < d.analyzed_files - matched {
        Some(format!(
            "only {matched} of {} analyzed files matched the coverage report",
            d.analyzed_files
        ))
    } else {
        None
    }
}

fn path_warnings(
    assignment: &Assignment,
    sources: &[&str],
    keys: &[&str],
    diagnostics: &ScopeDiagnostics,
) -> Vec<String> {
    let mut messages: Vec<_> = assignment
        .ties
        .iter()
        .map(|tie| {
            let spellings: Vec<_> = tie.keys.iter().map(|&k| keys[k]).collect();
            let mut owners: Vec<_> = tie.sources.iter().map(|&i| sources[i]).collect();
            owners.sort_unstable_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            format!(
                "coverage key {} is ambiguous between {} and was left unmatched",
                spellings.join(", "),
                owners.join(", ")
            )
        })
        .collect();
    messages.sort_unstable_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    messages.extend(overlap_warning(diagnostics));
    messages
}

fn merge_lines<'a>(
    files: &'a [StrictLcovFile],
    keys: &[usize],
) -> Cow<'a, BTreeMap<u64, f64>> {
    if let &[key] = keys {
        return Cow::Borrowed(&files[key].lines);
    }
    let mut merged = BTreeMap::new();
    for &key in keys {
        for (&line, &hits) in &files[key].lines {
            *merged.entry(line).or_default() += hits;
        }
    }
    Cow::Owned(merged)
}

fn merge_statements<'a>(
    files: &'a [(String, Vec<Statement>)],
    keys: &[usize],
) -> Cow<'a, [Statement]> {
    if let &[key] = keys {
        return Cow::Borrowed(&files[key].1);
    }
    let mut merged: Vec<Statement> = Vec::new();
    let mut by_span = HashMap::new();
    for &key in keys {
        for &s in &files[key].1 {
            // The reader normalizes signed zero, so bits identify every validated coordinate exactly.
            let span = (
                s.start.line.to_bits(),
                s.start.column.to_bits(),
                s.end.line.to_bits(),
                s.end.column.map(f64::to_bits),
            );
            let i = *by_span.entry(span).or_insert_with(|| {
                let i = merged.len();
                merged.push(Statement { hits: 0.0, ..s });
                i
            });
            merged[i].hits += s.hits;
        }
    }
    Cow::Owned(merged)
}

#[derive(Clone, Default)]
struct Evidence {
    covered: usize,
    total: usize,
    fully_hit_lines: FileCoverage,
}

impl Evidence {
    fn record(
        &mut self,
        line: u32,
        hits: f64,
    ) {
        let hit = hits > 0.0;
        self.total += 1;
        self.covered += usize::from(hit);
        let fully_hit = self.fully_hit_lines.lines.entry(line).or_insert(1);
        if !hit {
            *fully_hit = 0;
        }
    }

    fn percent(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (100.0 * self.covered as f64) / self.total as f64
        }
    }

    fn score(
        &self,
        row: &Row,
        matched: bool,
        missing: MissingCoveragePolicy,
    ) -> Option<AttributedRow> {
        let coverage = matched.then(|| self.percent());
        let percent = match (coverage, missing) {
            (Some(percent), _) => percent,
            (None, MissingCoveragePolicy::Pessimistic) => 0.0,
            (None, MissingCoveragePolicy::Optimistic) => 100.0,
            (None, MissingCoveragePolicy::Skip) => return None,
        };
        let cyclomatic = f64::from(row.cyclomatic);
        let entry = CrapEntry {
            file: row.file.clone(),
            function: row.name.clone(),
            line: row.span.start.line as usize,
            cyclomatic,
            coverage,
            crap: crap(cyclomatic, percent),
            uncovered: self
                .fully_hit_lines
                .uncovered_ranges_in_span(0, u32::MAX as usize),
            ..CrapEntry::default()
        };
        Some(AttributedRow {
            entry,
            covered: self.covered,
            total: self.total,
        })
    }
}

fn line_evidence(
    rows: &[&Row],
    lines: &BTreeMap<u64, f64>,
) -> Vec<Evidence> {
    rows.iter()
        .map(|row| {
            let mut evidence = Evidence::default();
            for line in row.statement_lines.iter().copied().collect::<BTreeSet<_>>() {
                if let Some(&hits) = lines.get(&u64::from(line)) {
                    evidence.record(line, hits);
                }
            }
            evidence
        })
        .collect()
}

impl From<Position> for Point {
    fn from(p: Position) -> Self {
        Self {
            line: f64::from(p.line),
            column: f64::from(p.column),
        }
    }
}

fn body_regions(rows: &[&Row]) -> Vec<(SourceSpan, usize)> {
    let mut bodies: Vec<_> = rows
        .iter()
        .enumerate()
        .flat_map(|(owner, row)| row.body_regions.iter().map(move |&body| (body, owner)))
        .collect();
    bodies.sort_by_key(|(body, _)| (body.start, Reverse(body.end)));
    bodies
}

fn statement_evidence(
    rows: &[&Row],
    statements: &[Statement],
) -> Vec<Evidence> {
    let bodies = body_regions(rows);
    let mut evidence = vec![Evidence::default(); rows.len()];
    for s in statements {
        if let Some(&(_, owner)) = bodies
            .iter()
            .rev()
            .find(|(body, _)| Point::from(body.start) <= s.start && s.start < Point::from(body.end))
        {
            evidence[owner].record(s.start.line as u32, s.hits);
        }
    }
    evidence
}

/// Scope counts and overlap warnings include every file with a row, even when missing rows are skipped.
#[must_use]
pub fn attribute(
    cwd: &Path,
    analyses: &[FileAnalysis],
    coverage: &Coverage,
    missing: MissingCoveragePolicy,
) -> Attribution {
    let files = group_rows(analyses);
    let sources: Vec<_> = files.iter().map(|file| file.name.as_ref()).collect();
    let keys = coverage.keys();
    let assignment = paths::assign_by_key(cwd, &sources, &keys, TiePolicy::Unmatched);
    let diagnostics = diagnostics(&assignment, &sources, &keys);
    let warnings = path_warnings(&assignment, &sources, &keys, &diagnostics);
    let mut rows = Vec::new();
    for (file, bound) in files.iter().zip(&assignment.bindings) {
        let evidence = match &coverage.0 {
            CoverageData::Lines(line_files) => {
                line_evidence(&file.rows, &merge_lines(line_files, bound))
            },
            CoverageData::Statements(statement_files) => {
                statement_evidence(&file.rows, &merge_statements(statement_files, bound))
            },
        };
        rows.extend(
            file.rows
                .iter()
                .zip(evidence)
                .filter_map(|(row, observed)| observed.score(row, !bound.is_empty(), missing)),
        );
    }
    Attribution {
        rows,
        unit: coverage.unit(),
        diagnostics,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{CoverageData, read_coverage};
    use crate::{Mode, analyze};
    use cargo_crap_core::coverage::{LineRange, read_strict_lcov};
    use proptest::prelude::*;

    #[test]
    fn winning_original_keys_combine_counts_per_line_and_complete_span() {
        let lcov = "SF:src/a.ts\nDA:1,1\nSF:src/./a.ts\nDA:1,2\nSF:/foreign/src/a.ts\nDA:1,3\nDA:2,0\nSF:D:\\ci\\src\\a.ts\nDA:2,5\nSF:foreign/src/a.ts\nDA:1,100";
        let files = read_strict_lcov(lcov, Path::new("cov")).unwrap();
        let keys: Vec<_> = files.iter().map(|f| f.key.as_str()).collect();
        let binding =
            paths::assign_by_key(Path::new("."), &["src/a.ts"], &keys, TiePolicy::Unmatched);
        assert_eq!(
            *merge_lines(&files, &binding.bindings[0]),
            BTreeMap::from([(1, 6.0), (2, 5.0)])
        );
        let cwd = tempfile::tempdir().unwrap();
        let path = cwd.path().join("alias");
        let span = r#"{"start":{"line":2,"column":2},"end":{"line":2,"column":5}}"#;
        let empty_end = span.replace(r#""column":5"#, r#""column":null"#);
        let report = format!(
            r#"{{"src/a.ts":{{"statementMap":{{"a":{span},"b":{span}}},"s":{{"a":0,"b":1}}}},
            "./src/a.ts":{{"statementMap":{{"a":{span},"b":{empty_end}}},"s":{{"a":3,"b":0}}}}}}"#
        );
        std::fs::write(&path, report).unwrap();
        let data = read_coverage(cwd.path(), &path).unwrap();
        let CoverageData::Statements(files) = data.0 else {
            panic!("expected statements")
        };
        let observed = |keys: &[usize]| {
            merge_statements(&files, keys)
                .iter()
                .map(|s| (s.end.column, s.hits))
                .collect::<Vec<_>>()
        };
        assert_eq!(observed(&[0]), [(Some(5.0), 0.0), (Some(5.0), 1.0)]);
        assert_eq!(observed(&[0, 1]), [(Some(5.0), 4.0), (None, 0.0)]);
        assert_eq!(observed(&[1, 0]), [(Some(5.0), 4.0), (None, 0.0)]);
    }

    #[test]
    fn repeated_analyses_count_distinct_files_and_keep_all_rows() {
        let analyses: Vec<_> = ["function f() {}", "function g() {}"]
            .iter()
            .map(|source| analyze(Path::new("same.ts"), source.as_bytes(), Mode::Ordinary).unwrap())
            .collect();
        let data = Coverage(CoverageData::Lines(
            read_strict_lcov("SF:same.ts", Path::new("cov")).unwrap(),
        ));
        let result = attribute(
            Path::new("."),
            &analyses,
            &data,
            MissingCoveragePolicy::Skip,
        );
        assert_eq!(result.diagnostics.analyzed_files, 1);
        assert_eq!(result.diagnostics.matched_files, 1);
        assert_eq!(
            result
                .rows
                .iter()
                .map(|r| r.entry.function.as_str())
                .collect::<Vec<_>>(),
            ["f", "g"]
        );
    }

    #[test]
    fn ambiguity_warnings_sort_sources_and_messages_by_utf16() {
        let files = [
            "\u{e000}/\u{e000}.ts",
            "\u{10000}/\u{e000}.ts",
            "\u{e000}/\u{10000}.ts",
            "\u{10000}/\u{10000}.ts",
        ];
        let analyses: Vec<_> = files
            .iter()
            .map(|name| analyze(Path::new(name), b"function f() {}", Mode::Ordinary).unwrap())
            .collect();
        let data = Coverage(CoverageData::Lines(
            read_strict_lcov("SF:\u{e000}.ts\nSF:\u{10000}.ts", Path::new("cov")).unwrap(),
        ));
        let result = attribute(
            Path::new("."),
            &analyses,
            &data,
            MissingCoveragePolicy::Skip,
        );
        for (index, ch) in ['\u{10000}', '\u{e000}'].into_iter().enumerate() {
            assert_eq!(
                result.warnings[index],
                format!(
                    "coverage key {ch}.ts is ambiguous between \u{10000}/{ch}.ts, \u{e000}/{ch}.ts and was left unmatched"
                )
            );
        }
    }

    proptest! {
        #[test]
        fn generated_lcov_reports_measure_each_owned_line_once(
            records in prop::collection::vec((1..10u32, 0..4u32, any::<bool>(), any::<u32>()), 0..30)
        ) {
            use std::fmt::Write as _;
            let cwd = tempfile::tempdir().unwrap();
            let mut analysis = analyze(Path::new("a.ts"), b"function f() {\n  before();\n  items.map(x => x);\n  after();\n  finish();\n}", Mode::Ordinary).unwrap();
            for row in &mut analysis.rows { row.statement_lines.extend_from_within(..); }
            let mut sums = BTreeMap::new();
            for &(line,hits,_,_) in &records {
                *sums.entry(line).or_insert(0u32) += hits;
            }
            let mut permuted = records.clone();
            permuted.sort_by_key(|r|r.3);
            for items in [records,permuted] {
                let mut text = "SF:a.ts\n".to_owned();
                for (line,hits,alias,_) in items {
                    let key = if alias { "./a.ts" } else { "a.ts" };
                    writeln!(text,"SF:{key}\nDA:{line},{hits}\nend_of_record").unwrap();
                }
                let path = cwd.path().join("cov");
                std::fs::write(&path,text).unwrap();
                let data = read_coverage(cwd.path(),&path).unwrap();
                let result = attribute(cwd.path(),std::slice::from_ref(&analysis),&data,MissingCoveragePolicy::Skip);
                prop_assert_eq!(result.rows.len(),2);
                for (row,owned) in result.rows.iter().zip([&[2,3,4,5][..], &[3]]) {
                    let observed: Vec<_> = sums.iter().filter(|(k,_)| owned.contains(k)).map(|(&line,&hits)|(line,hits)).collect();
                    let ranges: Vec<_> = observed.split(|&(_,hits)|hits>0).filter_map(|group|
                        Some(LineRange{start:group.first()?.0,end:group.last()?.0})).collect();
                    prop_assert_eq!(row.total,observed.len());
                    let hit_count = observed.iter().filter(|(_, hits)| *hits > 0).count();
                    prop_assert_eq!(row.covered, hit_count);
                    let percent = if observed.is_empty() { 0.0 } else { (100.0 * hit_count as f64) / observed.len() as f64 };
                    prop_assert_eq!(row.entry.coverage, Some(percent));
                    prop_assert_eq!(&row.entry.uncovered,&ranges);
                }
            }
        }
    }
}
