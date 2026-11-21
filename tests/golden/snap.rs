//! Verify mode reads each `.snap` file and compares its body with the run's
//! document itself, never through an insta assertion, so no `INSTA_UPDATE`
//! value can make a verifying run write or accept a golden. Record mode
//! (`GOLDEN_RECORD=1`) hands the document to insta under `INSTA_UPDATE=new`,
//! which writes a `.snap.new` for `cargo insta review` and leaves the `.snap`
//! alone. Setting a variable in this process is unsound while tests run on
//! threads, so a recording run without `INSTA_UPDATE=new` re-runs its test
//! binary with it set.

use std::collections::BTreeSet;
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use crate::encode::{Run, encode, matches};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Verify,
    Record,
    Relaunch,
}

/// insta 1.49 turns `INSTA_UPDATE=new` into `force` when either is set,
/// which would overwrite the accepted snapshot.
const FORCE_UPDATE: [&str; 2] = ["INSTA_FORCE_UPDATE", "INSTA_FORCE_UPDATE_SNAPSHOTS"];

pub fn mode(env: &dyn Fn(&str) -> Option<String>) -> Result<Mode, String> {
    match env("GOLDEN_RECORD").as_deref() {
        None => Ok(Mode::Verify),
        Some("1") => {
            if let Some(force) = FORCE_UPDATE.iter().find(|key| env(key).is_some()) {
                return Err(format!(
                    "GOLDEN_RECORD=1 refuses {force}: it overwrites accepted snapshots"
                ));
            }
            match (env("INSTA_UPDATE").as_deref(), env(PENDING_DIR)) {
                (Some("new"), None) => Ok(Mode::Record),
                _ => Ok(Mode::Relaunch),
            }
        },
        Some(other) => Err(format!("GOLDEN_RECORD is 1 or unset, found {other:?}")),
    }
}

/// insta writes pending snapshots there instead of beside the accepted
/// ones, where `cargo insta review` and this harness look for them.
const PENDING_DIR: &str = "INSTA_PENDING_DIR";

pub fn relaunch(args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(std::env::current_exe().expect("test binary"));
    command
        .args(args)
        .env("GOLDEN_RECORD", "1")
        .env("INSTA_UPDATE", "new");
    for key in FORCE_UPDATE.iter().chain(&[PENDING_DIR]) {
        command.env_remove(key);
    }
    let output = command.output().expect("re-running the test binary");
    // With --exact, a test name that matches nothing runs zero tests and
    // passes, which would make a recording run silently record nothing.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "the relaunch ran no test: {args:?}\n{stdout}"
    );
    output
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Missing {
        case: String,
        path: PathBuf,
    },
    Malformed {
        case: String,
        path: PathBuf,
    },
    Unreadable {
        case: String,
        path: PathBuf,
        error: String,
    },
    Changed {
        case: String,
        first_difference: String,
    },
}

impl fmt::Display for Failure {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Missing { case, path } => write!(f, "{case}: no snapshot {}", path.display()),
            Self::Unreadable { case, path, error } => {
                write!(f, "{case}: cannot read {}: {error}", path.display())
            },
            Self::Malformed { case, path } => {
                write!(f, "{case}: {} is not an insta snapshot", path.display())
            },
            Self::Changed {
                case,
                first_difference,
            } => write!(
                f,
                "{case}: output differs from its golden: {first_difference}"
            ),
        }
    }
}

/// Every failure of the cases `ids` against `snapshots` in `mode`, with
/// `run` producing each case's checked run: what the golden test asserts
/// empty. A run that fails its output checks is neither compared nor recorded.
pub fn check(
    mode: Mode,
    snapshots: &Snapshots,
    ids: &[&str],
    run: &dyn Fn(&str) -> Result<Run, Vec<String>>,
) -> Vec<String> {
    let mut failures = snapshots.check_set(ids, mode == Mode::Verify);
    // An unreadable directory is one failure, which check_set reports.
    let recorded = snapshots.recorded().unwrap_or_default();
    for id in ids {
        if mode == Mode::Verify && !recorded.contains(*id) {
            continue;
        }
        let run = match run(id) {
            Ok(run) => run,
            Err(problems) => {
                failures.extend(problems.into_iter().map(|p| format!("{id}: {p}")));
                continue;
            },
        };
        let outcome = match mode {
            Mode::Verify => snapshots.verify(id, &run).map_err(|f| f.to_string()),
            Mode::Record => snapshots.record(id, &run),
            Mode::Relaunch => Err(format!("{id}: record through snap::relaunch")),
        };
        failures.extend(outcome.err());
    }
    failures
}

pub struct Snapshots {
    dir: PathBuf,
}

impl Snapshots {
    pub fn committed() -> Self {
        Self::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"))
    }

    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(
        &self,
        case: &str,
    ) -> PathBuf {
        self.dir.join(format!("golden__{case}.snap"))
    }

    pub fn verify(
        &self,
        case: &str,
        run: &Run,
    ) -> Result<(), Failure> {
        let path = self.path(case);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Failure::Missing {
                    case: case.to_owned(),
                    path,
                });
            },
            Err(e) => {
                return Err(Failure::Unreadable {
                    case: case.to_owned(),
                    path,
                    error: e.to_string(),
                });
            },
        };
        let Some(recorded) = body(&text) else {
            return Err(Failure::Malformed {
                case: case.to_owned(),
                path,
            });
        };
        if matches(case, recorded, run) {
            return Ok(());
        }
        Err(Failure::Changed {
            case: case.to_owned(),
            first_difference: first_difference(recorded, &encode(case, run)),
        })
    }

    /// Hands the run to insta, which writes a pending snapshot when it
    /// differs from the accepted one. Returns insta's complaint, if any.
    pub fn record(
        &self,
        case: &str,
        run: &Run,
    ) -> Result<(), String> {
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(&self.dir);
        settings.set_prepend_module_to_snapshot(false);
        settings.set_omit_expression(true);
        let name = format!("golden__{case}");
        let document = encode(case, run);
        settings
            .bind(|| {
                std::panic::catch_unwind(AssertUnwindSafe(|| {
                    insta::assert_snapshot!(name, document);
                }))
            })
            .map_err(|panic| {
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| format!("{case}: insta rejected the run"))
            })
    }

    pub fn check_set(
        &self,
        cases: &[&str],
        missing_counts: bool,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        let mut listed = BTreeSet::new();
        for case in cases {
            if !listed.insert(*case) {
                problems.push(format!("case {case} is listed twice"));
            }
        }
        let recorded = match self.recorded() {
            Ok(recorded) => recorded,
            Err(problem) => return vec![problem],
        };
        for case in recorded.iter().filter(|c| !listed.contains(c.as_str())) {
            problems.push(format!("golden__{case}.snap: no case owns it"));
        }
        if missing_counts {
            for case in listed.iter().filter(|c| !recorded.contains(**c)) {
                problems.push(format!("{case}: no snapshot {}", self.path(case).display()));
            }
        }
        problems
    }

    fn recorded(&self) -> Result<BTreeSet<String>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(e) => return Err(format!("cannot read {}: {e}", self.dir.display())),
        };
        Ok(entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                Some(
                    name.strip_prefix("golden__")?
                        .strip_suffix(".snap")?
                        .to_owned(),
                )
            })
            .collect())
    }
}

fn body(snapshot: &str) -> Option<&str> {
    let after = snapshot.strip_prefix("---\n")?;
    let (_, document) = after.split_once("\n---\n")?;
    Some(document.strip_suffix('\n').unwrap_or(document))
}

fn first_difference(
    recorded: &str,
    actual: &str,
) -> String {
    let (mut golden, mut run) = (recorded.lines(), actual.lines());
    for number in 1.. {
        match (golden.next(), run.next()) {
            (None, None) => break,
            (a, b) if a == b => {},
            (a, b) => {
                let show = |line: Option<&str>| {
                    line.map_or_else(|| "<none>".to_owned(), |l| format!("{l:?}"))
                };
                return format!("line {number}: golden {}, run {}", show(a), show(b));
            },
        }
    }
    "the documents differ only in line endings".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::Status;

    fn sample(stdout: &str) -> Run {
        Run {
            stdout: stdout.as_bytes().to_vec(),
            stderr: b"warning: x\n".to_vec(),
            status: Status::Exit(0),
        }
    }

    fn insta_file(document: &str) -> String {
        format!("---\nsource: tests/golden/snap.rs\n---\n{document}\n")
    }

    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut all: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .expect("snapshot dir")
            .map(|e| {
                let e = e.expect("entry");
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).expect("read"),
                )
            })
            .collect();
        all.sort();
        all
    }

    fn stale_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("dir");
        for (case, stdout) in [("changed", "old table\n"), ("same", "same table\n")] {
            let doc = encode(case, &sample(stdout));
            std::fs::write(
                dir.path().join(format!("golden__{case}.snap")),
                insta_file(&doc),
            )
            .expect("write");
        }
        dir
    }

    const CASES: [&str; 3] = ["changed", "missing", "same"];

    fn fresh(case: &str) -> Run {
        sample(if case == "same" {
            "same table\n"
        } else {
            "new table\n"
        })
    }

    #[test]
    fn a_run_that_fails_its_checks_is_reported_and_not_compared() {
        let dir = stale_dir();
        let failures = check(
            Mode::Verify,
            &Snapshots::at(dir.path().into()),
            &["changed", "same"],
            &|case| {
                if case == "changed" {
                    Err(vec!["stdout names the path /tmp".into()])
                } else {
                    Ok(fresh(case))
                }
            },
        );
        assert_eq!(failures, ["changed: stdout names the path /tmp"]);
    }

    const UPDATE_VALUES: [&str; 5] = ["always", "force", "unseen", "new", "auto"];
    const SNAP_DIR: &str = "GOLDEN_SNAP_DIR";
    const RESULT: &str = "GOLDEN_SNAP_RESULT";

    fn rerun(
        inner: &str,
        env: &[(&str, &str)],
    ) -> std::process::Output {
        let mut command = std::process::Command::new(std::env::current_exe().expect("test binary"));
        command.args([inner, "--exact", "--ignored"]);
        for key in ["GOLDEN_RECORD", "INSTA_UPDATE", PENDING_DIR]
            .iter()
            .chain(&FORCE_UPDATE)
        {
            command.env_remove(key);
        }
        command
            .envs(env.iter().copied())
            .output()
            .expect("re-running the test binary")
    }

    #[test]
    fn a_matching_run_verifies() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = encode("p_human", &sample("table\n"));
        std::fs::write(dir.path().join("golden__p_human.snap"), insta_file(&doc)).expect("write");
        assert_eq!(
            Snapshots::at(dir.path().into()).verify("p_human", &sample("table\n")),
            Ok(())
        );
    }

    #[test]
    fn a_headerless_snapshot_is_malformed() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = encode("c", &sample("table\n"));
        std::fs::write(dir.path().join("golden__c.snap"), &doc).expect("write");
        assert!(matches!(
            Snapshots::at(dir.path().into()).verify("c", &sample("table\n")),
            Err(Failure::Malformed { .. })
        ));
    }

    #[test]
    fn a_body_containing_a_rule_line_is_read_whole() {
        let doc = "case: c\nstdout:\n---\nrule\n<<end>>\nstderr:\n<<end>>\nexit: 0";
        assert_eq!(body(&insta_file(doc)), Some(doc));
        assert_eq!(body("no header"), None);
        assert_eq!(body("---\nsource: x\n"), None);
    }

    #[test]
    fn verification_never_writes_a_golden() {
        for value in UPDATE_VALUES {
            let dir = stale_dir();
            let before = files(dir.path());
            let result = tempfile::NamedTempFile::new().expect("result");
            let dir_text = dir.path().to_str().expect("utf-8 dir");
            let result_text = result.path().to_str().expect("utf-8 path");
            let child = rerun(
                "snap::tests::verify_under_the_hosts_insta_update",
                &[
                    ("INSTA_UPDATE", value),
                    (SNAP_DIR, dir_text),
                    (RESULT, result_text),
                ],
            );
            assert!(
                child.status.success(),
                "{value}: {}",
                String::from_utf8_lossy(&child.stdout)
            );
            let report = std::fs::read_to_string(result.path()).expect("report");
            assert!(
                report.contains("changed: output differs"),
                "{value}: {report}"
            );
            assert!(report.contains("missing: no snapshot"), "{value}: {report}");
            assert_eq!(
                files(dir.path()),
                before,
                "INSTA_UPDATE={value} wrote a snapshot"
            );
        }
    }

    #[test]
    #[ignore = "run by verification_never_writes_a_golden"]
    fn verify_under_the_hosts_insta_update() {
        let (Some(dir), Some(result)) = (std::env::var_os(SNAP_DIR), std::env::var_os(RESULT))
        else {
            return;
        };
        let mode = mode(&|key| std::env::var(key).ok()).expect("a mode");
        let report = check(mode, &Snapshots::at(dir.into()), &CASES, &|case| {
            Ok(fresh(case))
        });
        std::fs::write(result, report.join("\n")).expect("report");
    }

    #[test]
    fn recording_writes_pending_snapshots_for_review() {
        let elsewhere = tempfile::tempdir().expect("a pending dir");
        let elsewhere = elsewhere.path().to_str().expect("utf-8 dir").to_owned();
        for (update, pending) in [
            (None, None),
            (Some("always"), None),
            (Some("new"), None),
            (Some("new"), Some(elsewhere.as_str())),
        ] {
            recording_under(update, pending);
        }
    }

    fn recording_under(
        update: Option<&str>,
        pending: Option<&str>,
    ) {
        let dir = stale_dir();
        let accepted = std::fs::read(dir.path().join("golden__changed.snap")).expect("accepted");
        let dir_text = dir.path().to_str().expect("utf-8 dir");
        let mut env = vec![("GOLDEN_RECORD", "1"), (SNAP_DIR, dir_text)];
        env.extend(update.map(|u| ("INSTA_UPDATE", u)));
        env.extend(pending.map(|p| (PENDING_DIR, p)));
        let child = rerun("snap::tests::record_into_the_given_dir", &env);
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stdout)
        );
        for case in ["changed", "missing"] {
            let pending =
                std::fs::read_to_string(dir.path().join(format!("golden__{case}.snap.new")))
                    .unwrap_or_else(|e| panic!("{case}: no pending snapshot: {e}"));
            assert_eq!(
                body(&pending),
                Some(encode(case, &sample("new table\n")).as_str())
            );
        }
        assert_eq!(
            std::fs::read(dir.path().join("golden__changed.snap")).expect("accepted"),
            accepted
        );
        assert!(!dir.path().join("golden__missing.snap").exists());
        assert!(
            !dir.path().join("golden__same.snap.new").exists(),
            "a matching run left a pending snapshot"
        );
        std::fs::rename(
            dir.path().join("golden__missing.snap.new"),
            dir.path().join("golden__missing.snap"),
        )
        .expect("accept");
        assert_eq!(
            Snapshots::at(dir.path().into()).verify("missing", &sample("new table\n")),
            Ok(())
        );
    }

    #[test]
    #[ignore = "run by recording_writes_pending_snapshots_for_review"]
    fn record_into_the_given_dir() {
        let Some(dir) = std::env::var_os(SNAP_DIR) else {
            return;
        };
        match mode(&|key| std::env::var(key).ok()).expect("a mode") {
            Mode::Relaunch => {
                let child = relaunch(&[
                    "snap::tests::record_into_the_given_dir",
                    "--exact",
                    "--ignored",
                ]);
                assert!(
                    child.status.success(),
                    "{}",
                    String::from_utf8_lossy(&child.stdout)
                );
            },
            Mode::Record => {
                let failures = check(Mode::Record, &Snapshots::at(dir.into()), &CASES, &|case| {
                    Ok(fresh(case))
                });
                assert_eq!(failures.len(), 2, "{failures:?}");
            },
            Mode::Verify => panic!("GOLDEN_RECORD=1 did not record"),
        }
    }

    #[test]
    fn a_relaunch_that_runs_no_test_fails() {
        let outcome = std::panic::catch_unwind(|| relaunch(&["no_such_test", "--exact"]));
        assert!(outcome.is_err(), "a relaunch matching no test passed");
    }

    #[test]
    fn golden_record_alone_selects_recording() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(mode(&env(&[])), Ok(Mode::Verify));
        assert_eq!(mode(&env(&[("INSTA_UPDATE", "always")])), Ok(Mode::Verify));
        assert_eq!(
            mode(&env(&[("GOLDEN_RECORD", "1"), ("INSTA_UPDATE", "new")])),
            Ok(Mode::Record)
        );
        assert_eq!(mode(&env(&[("GOLDEN_RECORD", "1")])), Ok(Mode::Relaunch));
        assert_eq!(
            mode(&env(&[
                ("GOLDEN_RECORD", "1"),
                ("INSTA_UPDATE", "new"),
                (PENDING_DIR, "/elsewhere")
            ])),
            Ok(Mode::Relaunch)
        );
        for update in ["always", "force", "unseen", "no", "auto"] {
            let pairs: &'static [(&str, &str)] =
                Box::leak(Box::new([("GOLDEN_RECORD", "1"), ("INSTA_UPDATE", update)]));
            assert_eq!(
                mode(&env(pairs)),
                Ok(Mode::Relaunch),
                "INSTA_UPDATE={update}"
            );
        }
        for force in FORCE_UPDATE {
            let pairs: &'static [(&str, &str)] = Box::leak(Box::new([
                ("GOLDEN_RECORD", "1"),
                ("INSTA_UPDATE", "new"),
                (force, "1"),
            ]));
            assert!(mode(&env(pairs)).is_err(), "record mode accepted {force}");
        }
        assert!(mode(&env(&[("GOLDEN_RECORD", "0"), ("INSTA_UPDATE", "new")])).is_err());
    }

    #[test]
    fn a_missing_snapshot_is_reported_once() {
        let dir = stale_dir();
        let failures = check(
            Mode::Verify,
            &Snapshots::at(dir.path().into()),
            &CASES,
            &|case| Ok(fresh(case)),
        );
        let missing = failures
            .iter()
            .filter(|f| f.starts_with("missing:"))
            .count();
        assert_eq!(missing, 1, "{failures:?}");
    }

    #[test]
    fn an_unreadable_snapshot_dir_is_one_failure_naming_it() {
        let not_a_dir = tempfile::NamedTempFile::new().expect("a file where the directory goes");
        let snapshots = Snapshots::at(not_a_dir.path().into());
        let failures = check(Mode::Verify, &snapshots, &CASES, &|case| Ok(fresh(case)));
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].starts_with("cannot read "), "{failures:?}");
    }

    #[test]
    fn an_unreadable_snapshot_is_not_called_missing() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::create_dir(dir.path().join("golden__c.snap"))
            .expect("a directory where the file goes");
        assert!(matches!(
            Snapshots::at(dir.path().into()).verify("c", &sample("table\n")),
            Err(Failure::Unreadable { .. })
        ));
    }

    #[test]
    fn the_snapshot_files_equal_the_case_list() {
        let dir = tempfile::tempdir().expect("dir");
        for name in [
            "golden__a.snap",
            "golden__orphan.snap",
            "width_snapshots__x.snap",
            "golden__a.snap.new",
        ] {
            std::fs::write(dir.path().join(name), "").expect("write");
        }
        let snapshots = Snapshots::at(dir.path().into());
        let problems = snapshots.check_set(&["a", "b", "a"], true);
        assert!(
            problems.iter().any(|p| p.contains("golden__orphan.snap")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("case a is listed twice")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("b: no snapshot")),
            "{problems:?}"
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(
            snapshots.check_set(&["a", "b", "orphan"], false),
            Vec::<String>::new(),
            "record mode reports no missing snapshots"
        );
    }

    #[test]
    fn a_changed_run_names_its_first_differing_line() {
        assert_eq!(
            first_difference("a\nb\nc", "a\nx\nc"),
            "line 2: golden \"b\", run \"x\""
        );
        assert_eq!(
            first_difference("a", "a\nb"),
            "line 2: golden <none>, run \"b\""
        );
    }
}
