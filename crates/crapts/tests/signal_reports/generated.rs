use super::{Project, support};
use cargo_crap_core::coverage::LineRange;
use cargo_crap_core::signals::Signal;
use crapts::{Mode, Row, Side, analyze};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;

#[derive(Clone, Debug)]
enum Statement {
    Leaf(&'static str),
    Conditional(&'static str),
    Block(Box<Self>),
    If(&'static str, Box<Self>, Box<Self>),
    While(&'static str, Box<Self>),
    Callback(Box<Self>),
}

fn statement(ts: bool) -> impl Strategy<Value = Statement> {
    let mut leaves = vec!["return x;", "debugger;", "use(x);"];
    if ts {
        leaves.push("use(x!);");
    }
    let leaf = prop::sample::select(leaves).prop_map(Statement::Leaf);
    leaf.prop_recursive(4, 64, 4, |inner| {
        let predicate = prop::sample::select(vec!["x", "x && y", "(x ?? y) || z", "x && y || z"]);
        prop_oneof![
            inner.clone().prop_map(|a| Statement::Block(Box::new(a))),
            (predicate.clone(), inner.clone(), inner.clone()).prop_map(|(p, a, b)| Statement::If(
                p,
                Box::new(a),
                Box::new(b)
            )),
            (predicate.clone(), inner.clone()).prop_map(|(p, a)| Statement::While(p, Box::new(a))),
            predicate.prop_map(Statement::Conditional),
            inner.prop_map(|a| Statement::Callback(Box::new(a))),
        ]
    })
}

#[derive(Default)]
struct Owner {
    start: u32,
    end: u32,
    chunks: Vec<LineRange>,
    operators: usize,
}

#[derive(Default)]
struct Source {
    lines: Vec<String>,
    owners: Vec<Owner>,
    statements: Vec<LineRange>,
}

impl Source {
    fn line(
        &mut self,
        text: &str,
    ) -> u32 {
        self.lines.push(text.into());
        u32::try_from(self.lines.len()).unwrap()
    }

    fn callable(
        &mut self,
        header: &str,
        body: &[Statement],
        close: &str,
    ) {
        let start = self.line(header);
        let owner = self.owners.len();
        self.owners.push(Owner {
            start,
            ..Owner::default()
        });
        for node in body {
            let chunk = self.emit(node, owner);
            self.owners[owner].chunks.push(chunk);
        }
        let end = self.line(close);
        self.owners[owner].end = end;
    }

    fn emit(
        &mut self,
        node: &Statement,
        owner: usize,
    ) -> LineRange {
        let start = u32::try_from(self.lines.len()).unwrap() + 1;
        match node {
            Statement::Leaf(s) => {
                self.line(s);
                self.record_ops(owner, s);
            },
            Statement::Conditional(test) => {
                self.line(&format!("return {test} ? a : b;"));
                self.record_ops(owner, test);
            },
            Statement::Block(s) => {
                self.line("{");
                self.emit(s, owner);
                self.line("}");
            },
            Statement::If(test, a, b) => {
                self.line(&format!("if ({test}) {{"));
                self.record_ops(owner, test);
                self.emit(a, owner);
                self.line("} else {");
                self.emit(b, owner);
                self.line("}");
            },
            Statement::While(test, s) => {
                self.line(&format!("while ({test}) {{"));
                self.record_ops(owner, test);
                self.emit(s, owner);
                self.line("}");
            },
            Statement::Callback(s) => {
                self.callable("xs.map(x => {", std::slice::from_ref(s.as_ref()), "});");
            },
        }
        let range = LineRange {
            start,
            end: u32::try_from(self.lines.len()).unwrap(),
        };
        if !matches!(node, Statement::Block(_)) {
            self.statements.push(range);
        }
        range
    }

    fn record_ops(
        &mut self,
        owner: usize,
        test: &str,
    ) {
        self.owners[owner].operators += test.matches("&&").count() + test.matches("||").count();
    }

    fn text(
        &self,
        crlf: bool,
    ) -> String {
        self.lines.join(if crlf { "\r\n" } else { "\n" })
    }

    fn istanbul(
        &self,
        path: &str,
    ) -> Value {
        let mut map = json!({});
        let mut hits = json!({});
        for (i, span) in self.statements.iter().enumerate() {
            let index = i.to_string();
            let end = self.lines[span.end as usize - 1].len();
            map[&index] = json!({"start":{"line":span.start,"column":0},"end":{"line":span.end,"column":end}});
            hits[&index] = json!(1);
        }
        json!({(path):{"path":path,"statementMap":map,"s":hits,"fnMap":{},"f":{},"branchMap":{},"b":{}}})
    }
}

fn rows(
    path: &str,
    source: &str,
) -> Vec<Row> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
        .unwrap()
        .rows
}

fn fixture(body: &[Statement]) -> Source {
    let mut source = Source::default();
    source.callable("function f() {", body, "}");
    source
}

fn assert_evidence(
    source: &Source,
    actual: &[Row],
) {
    assert_eq!(actual.len(), source.owners.len());
    for row in actual {
        let owner = source
            .owners
            .iter()
            .find(|o| o.start == row.span.start.line)
            .unwrap();
        let m = &row.measured;
        assert_eq!(
            m.spans.lines,
            [LineRange {
                start: owner.start,
                end: owner.end
            }]
        );
        assert_eq!(m.signals.lines, owner.end - owner.start + 1);
        assert_eq!(m.signals.bumps as usize, m.spans.bumps.len());
        assert!(m.spans.bumps.len() <= owner.chunks.len());
        for signal in Signal::ALL.into_iter().filter(|s| *s != Signal::Lines) {
            for span in signal.spans(&m.spans) {
                assert!(span.start <= span.end);
                assert!(
                    owner
                        .chunks
                        .iter()
                        .any(|c| c.start <= span.start && span.end <= c.end)
                );
                if matches!(signal, Signal::Nesting | Signal::Bumps) {
                    assert!(owner.chunks.contains(span));
                } else {
                    assert!(source.owners.iter().all(|child| !(owner.start < child.start
                        && child.start < span.start
                        && span.end < child.end)));
                }
            }
        }
    }
}

fn scores(
    body: &[Statement],
    ts: bool,
    crlf: bool,
) {
    let project = Project(tempfile::tempdir().unwrap());
    let path = if ts { "f.ts" } else { "f.js" };
    let source = fixture(body);
    let text = source.text(crlf);
    let actual = rows(path, &text);
    assert_evidence(&source, &actual);
    project.write(path, &text);
    let mut lines = source
        .statements
        .iter()
        .map(|s| s.start)
        .collect::<std::collections::BTreeSet<_>>();
    // A file with no statements still needs a matching LCOV record.
    lines.insert(1);
    let mut hits = format!("SF:{path}\n");
    for line in lines {
        writeln!(hits, "DA:{line},1").unwrap();
    }
    hits.push_str("end_of_record\n");
    project.write("hits.info", hits);
    project.save("coverage.json", &source.istanbul(path));
    for coverage in ["hits.info", "coverage.json"] {
        project.config(false, "");
        let off = project.report(coverage, &[], 0);
        project.config(true, "");
        let mut on = project.report(coverage, &[], 0);
        assert_eq!(
            on["summary"]
                .as_object_mut()
                .unwrap()
                .remove("signal_regressions"),
            Some(Value::Null)
        );
        let reported = on["entries"].as_array_mut().unwrap();
        assert_eq!(reported.len(), actual.len());
        for entry in reported {
            let row = actual.iter().find(|r| entry["function"] == r.name).unwrap();
            let vector = entry.as_object_mut().unwrap().remove("signals");
            assert_eq!(vector, Some(json!(row.measured.signals)));
        }
        assert_eq!(on, off);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..support::proptest_config() })]
    #[test]
    fn s11_each_coverage_unit_preserves_scores_and_owned_evidence(
        (ts, body) in any::<bool>().prop_flat_map(|ts| (Just(ts), prop::collection::vec(statement(ts), 0..=4))),
        crlf in any::<bool>()
    ) {
        scores(&body, ts, crlf);
    }
}

#[test]
fn s11_empty_and_depth_four_bodies_cover_both_line_endings_and_languages() {
    let deep = Statement::While(
        "x",
        Box::new(Statement::Callback(Box::new(Statement::If(
            "x && y",
            Box::new(Statement::Block(Box::new(Statement::Leaf("debugger;")))),
            Box::new(Statement::Leaf("return x;")),
        )))),
    );
    for body in [&[][..], std::slice::from_ref(&deep)] {
        for ts in [true, false] {
            for crlf in [true, false] {
                scores(body, ts, crlf);
            }
        }
    }
}

fn check_wrapped(
    ts: bool,
    body: &[Statement],
    crlf: bool,
) -> proptest::test_runner::TestCaseResult {
    let path = if ts { "f.ts" } else { "f.js" };
    let original = fixture(body);
    let before = rows(path, &original.text(crlf));
    let mut wrapped = original.lines.clone();
    wrapped.insert(1, "if (guard) {".into());
    wrapped.push("}".into());
    let after = rows(path, &wrapped.join(if crlf { "\r\n" } else { "\n" }));
    let old = before[0].measured.signals;
    let new = after[0].measured.signals;
    prop_assert_eq!(new.nesting, old.nesting + 1);
    prop_assert_eq!(new.bumps, u32::from(old.nesting > 0));
    prop_assert_eq!(
        (old.bool_ops, old.stubs, old.panics),
        (new.bool_ops, new.stubs, new.panics)
    );
    for (row, owner) in before.iter().zip(&original.owners) {
        prop_assert!((row.measured.signals.bool_ops as usize) <= owner.operators);
    }
    let children = |rs: &[Row]| {
        rs.iter()
            .skip(1)
            .map(|r| r.measured.signals)
            .collect::<Vec<_>>()
    };
    prop_assert_eq!(children(&before), children(&after));
    Ok(())
}

proptest! {
    #![proptest_config(support::proptest_config())]
    #[test]
    fn s27_wrapping_adds_one_level_and_preserves_owned_evidence(
        (ts, body) in any::<bool>().prop_flat_map(|ts| (Just(ts), prop::collection::vec(statement(ts), 0..=4))),
        crlf in any::<bool>()
    ) {
        check_wrapped(ts, &body, crlf)?;
    }
}
