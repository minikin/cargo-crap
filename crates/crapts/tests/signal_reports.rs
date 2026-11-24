mod common;
#[path = "signal_reports/comparison.rs"]
mod comparison;
#[path = "signal_reports/generated.rs"]
mod generated;
mod support;

use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use tempfile::TempDir;

struct Project(TempDir);

impl Project {
    fn new() -> Self {
        let project = Self(tempfile::tempdir().unwrap());
        project.source("f.ts", "function f() { return 1 }\n");
        project
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn write(
        &self,
        name: &str,
        content: impl AsRef<[u8]>,
    ) {
        fs::write(self.root().join(name), content).unwrap();
    }

    fn source(
        &self,
        name: &str,
        source: &str,
    ) {
        use std::fmt::Write as _;
        self.write(name, source);
        let mut coverage = format!("SF:{name}\n");
        for number in 1..=source.lines().count() {
            writeln!(coverage, "DA:{number},1").unwrap();
        }
        coverage.push_str("end_of_record\n");
        self.write("hits.info", coverage);
    }

    fn config(
        &self,
        enabled: bool,
        policy: &str,
    ) {
        self.write(
            ".crapts.toml",
            format!("[signals]\nenabled = {enabled}\n{policy}"),
        );
    }

    fn run(
        &self,
        args: &[&str],
    ) -> (ExitCode, String, String) {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = crapts::cli::run(self.root(), &args, &mut stdout, &mut stderr);
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    fn report(
        &self,
        coverage: &str,
        flags: &[&str],
        expected: u8,
    ) -> Value {
        let mut args = vec!["--format", "json", "--coverage", coverage];
        args.extend_from_slice(flags);
        let (code, out, err) = self.run(&args);
        assert_eq!(code, ExitCode::from(expected), "{args:?} {err} {out}");
        if flags.contains(&"--baseline") {
            common::validated_delta(out.as_bytes())
        } else {
            common::validated_report(out.as_bytes())
        }
    }

    fn save(
        &self,
        name: &str,
        value: &Value,
    ) {
        self.write(name, serde_json::to_vec(value).unwrap());
    }

    fn envelopes(&self) -> [Value; 2] {
        let absolute = self.report("hits.info", &[], 0);
        self.save("base.json", &absolute);
        let delta = self.report("hits.info", &["--baseline", "base.json"], 0);
        [absolute, delta]
    }
}

fn vector() -> Value {
    json!({"nesting":2,"lines":1,"bumps":0,"bool_ops":0,"stubs":0,"panics":0})
}

fn admission(
    project: &Project,
    schema: &jsonschema::Validator,
    doc: &Value,
    error: Option<&str>,
) {
    assert_eq!(schema.is_valid(doc), error.is_none(), "{error:?}: {doc}");
    project.save("candidate.json", doc);
    project.write("out.txt", "keep");
    let (code, out, err) = project.run(&[
        "--coverage",
        "hits.info",
        "--baseline",
        "candidate.json",
        "--output",
        "out.txt",
    ]);
    assert!(out.is_empty(), "{out}");
    if let Some(field) = error {
        assert_eq!(code, ExitCode::from(2), "{doc}");
        assert!(
            err.contains("candidate.json") && err.contains(field),
            "{field}: {err}"
        );
        assert_eq!(fs::read(project.root().join("out.txt")).unwrap(), b"keep");
    } else {
        assert_eq!(code, ExitCode::SUCCESS, "{err}");
    }
}

fn check_vectors(
    project: &Project,
    schema: &jsonschema::Validator,
    envelope: &Value,
    slot: &str,
) {
    let check = |candidate: Value, error: Option<&str>| {
        let mut doc = envelope.clone();
        doc["entries"][0][slot] = candidate;
        admission(project, schema, &doc, error);
    };
    let v = vector();
    check(v.clone(), None);
    for (field, value) in [("nesting", json!(2.0)), ("panics", json!(u32::MAX))] {
        let mut candidate = v.clone();
        candidate[field] = value;
        check(candidate, None);
    }
    for bad in [
        json!({}),
        json!({"nesting":2}),
        Value::Null,
        json!([]),
        json!("2"),
    ] {
        check(bad, Some(slot));
    }
    let mut extra = v.clone();
    extra["depth"] = json!(1);
    check(extra, Some("depth"));
    let mut zero = v.clone();
    zero["lines"] = json!(0);
    check(zero, Some("lines"));
    for field in v.as_object().unwrap().keys() {
        let mut missing = v.clone();
        missing.as_object_mut().unwrap().remove(field);
        check(missing, Some(field));
        for invalid in [
            json!(-1),
            json!(0.5),
            json!("2"),
            json!(u64::from(u32::MAX) + 1),
        ] {
            let mut candidate = v.clone();
            candidate[field] = invalid;
            check(candidate, Some(field));
        }
    }
}

const SCHEMAS: [&str; 2] = [
    "schemas/crapts/report-v1.json",
    "schemas/crapts/delta-v1.json",
];

#[test]
fn s19_every_vector_slot_uses_the_same_closed_draft07_integer_contract() {
    let project = Project::new();
    for (index, mut doc) in project.envelopes().into_iter().enumerate() {
        let schema = common::compile(SCHEMAS[index]);
        check_vectors(&project, &schema, &doc, "signals");
        if index == 1 {
            doc["entries"][0]["signals"] = vector();
            check_vectors(&project, &schema, &doc, "baseline_signals");
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum Slot {
    Absolute,
    Paired,
    New,
    Removed,
}

impl Slot {
    fn document(
        self,
        docs: &[Value; 2],
    ) -> (Value, &'static str) {
        match self {
            Self::Absolute => (docs[0].clone(), "entries"),
            Self::Paired => (docs[1].clone(), "entries"),
            Self::New => {
                let mut doc = docs[1].clone();
                let row = &mut doc["entries"][0];
                row["status"] = json!("new");
                row["delta"] = Value::Null;
                row["baseline_crap"] = Value::Null;
                (doc, "entries")
            },
            Self::Removed => {
                let mut doc = docs[1].clone();
                doc["removed"] = json!([{"file":"gone.ts","function":"gone","baseline_crap":1}]);
                (doc, "removed")
            },
        }
    }
}

#[test]
fn s47_current_historical_and_internal_keys_are_admitted_only_at_their_owned_rows() {
    let project = Project::new();
    let docs = project.envelopes();
    for slot in [Slot::Absolute, Slot::Paired, Slot::New, Slot::Removed] {
        let schema = common::compile(SCHEMAS[usize::from(!matches!(slot, Slot::Absolute))]);
        let (sample, rows) = slot.document(&docs);
        for current in [false, true] {
            for historical in [false, true] {
                let mut doc = sample.clone();
                if current {
                    doc[rows][0]["signals"] = vector();
                }
                if historical {
                    doc[rows][0]["baseline_signals"] = vector();
                }
                let refusal = if matches!(slot, Slot::Removed) && current {
                    Some("signals")
                } else if historical && !(matches!(slot, Slot::Paired) && current) {
                    Some("baseline_signals")
                } else {
                    None
                };
                admission(&project, &schema, &doc, refusal);
            }
        }
        for key in ["signals", "baseline_signals", "signal_spans"] {
            let mut doc = sample.clone();
            doc[rows][0][key] = Value::Null;
            admission(&project, &schema, &doc, Some(key));
        }
    }
    let mut delta = docs[1].clone();
    delta["entries"][0]["signals"] = vector();
    let mut old = vector();
    old["lines"] = json!(99);
    delta["entries"][0]["baseline_signals"] = old;
    project.save("saved.json", &delta);
    project.config(true, "");
    let comparison =
        |project: &Project| project.report("hits.info", &["--baseline", "saved.json"], 0);
    let paired = comparison(&project);
    assert_eq!(paired["entries"][0]["baseline_signals"], vector());
    assert_eq!(paired["summary"]["signal_regressions"], 0);
    delta["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("signals");
    delta["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("baseline_signals");
    project.save("saved.json", &delta);
    let legacy = comparison(&project);
    assert_eq!(
        legacy["summary"].get("signal_regressions"),
        Some(&Value::Null)
    );
    assert!(legacy["entries"][0].get("baseline_signals").is_none());
}

#[test]
fn optional_summary_count_admits_null_or_nonnegative_delta_integers() {
    let project = Project::new();
    for (index, sample) in project.envelopes().into_iter().enumerate() {
        let schema = common::compile(SCHEMAS[index]);
        for (value, delta_ok) in [
            (Value::Null, true),
            (json!(0), true),
            (json!(2.0), true),
            (json!(-1), false),
            (json!(0.5), false),
            (json!("2"), false),
        ] {
            let accepted = value.is_null() || index == 1 && delta_ok;
            let mut doc = sample.clone();
            doc["summary"]["signal_regressions"] = value;
            admission(
                &project,
                &schema,
                &doc,
                (!accepted).then_some("signal_regressions"),
            );
        }
    }
}

#[test]
fn s57_opting_out_reads_a_modern_baseline_and_keeps_the_wire_quiet() {
    let project = Project::new();
    let [_, mut baseline] = project.envelopes();
    let measured = json!({"nesting":0,"lines":1,"bumps":0,"bool_ops":0,"stubs":0,"panics":0});
    baseline["entries"][0]["signals"] = measured.clone();
    baseline["entries"][0]["baseline_signals"] = measured;
    baseline["summary"]["signal_regressions"] = json!(0);
    project.save("modern.json", &baseline);
    project.source("f.ts", "function f() { debugger; return 1 }\n");
    project.config(false, "");
    let (code, out, err) = project.run(&[
        "--coverage",
        "hits.info",
        "--format",
        "json",
        "--baseline",
        "modern.json",
        "--fail-regression",
    ]);
    assert_eq!(code, ExitCode::SUCCESS, "{err}");
    assert!(err.is_empty(), "{err}");
    let doc = common::validated_delta(out.as_bytes());
    assert_eq!(doc["entries"][0]["status"], "unchanged");
    assert!(doc["summary"].get("signal_regressions").is_none());
    for key in ["signals", "baseline_signals", "signal_spans", "reasons"] {
        assert!(doc["entries"][0].get(key).is_none(), "{key}");
    }
}
