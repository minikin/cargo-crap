//! Golden outputs of the cargo-crap binary: every case's stdout, stderr and
//! exit status, recorded from main at `4b57930` (package version 0.9.0), so
//! a later change that moves code between crates proves it changed no byte.
//!
//! Unix only: Windows prints `.\src\lib.rs` and its own OS error text, and a
//! normaliser that hid those could hide a real change.
//!
//! Re-record a changed golden with `GOLDEN_RECORD=1 cargo nextest run --test
//! golden`, then review the pending snapshots with `cargo insta review`.

#![cfg(unix)]

#[path = "golden/cases.rs"]
mod cases;

#[path = "golden/checks.rs"]
mod checks;

#[path = "golden/config.rs"]
mod config;

#[path = "golden/encode.rs"]
mod encode;

#[path = "golden/run.rs"]
mod run;

#[path = "golden/snap.rs"]
mod snap;

#[path = "support/schemas.rs"]
mod schemas;

#[path = "golden/table.rs"]
mod table;

#[test]
fn every_case_matches_the_golden_recorded_from_main_at_4b57930() {
    let mode = snap::mode(&|key| std::env::var(key).ok()).unwrap_or_else(|e| panic!("{e}"));
    if mode == snap::Mode::Relaunch {
        let child = snap::relaunch(&[
            "every_case_matches_the_golden_recorded_from_main_at_4b57930",
            "--exact",
        ]);
        let output = String::from_utf8_lossy(&child.stdout);
        assert!(child.status.success(), "{output}");
        return;
    }
    let failures = verify_all(mode, &snap::Snapshots::committed(), &cases::all());
    assert!(
        failures.is_empty(),
        "{} golden failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn snapshots_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

/// Every failure of `cases` against `snapshots`, including a verifying run
/// that changed their directory.
fn verify_all(
    mode: snap::Mode,
    snapshots: &snap::Snapshots,
    cases: &[run::Case],
) -> Vec<String> {
    let dir = snapshots.dir();
    let ids: Vec<&str> = cases.iter().map(|case| case.id).collect();
    let run = |id: &str| {
        let case = cases
            .iter()
            .find(|case| case.id == id)
            .expect("a listed case");
        let (run, copy) = run::run(case)?;
        checks::finish(case, run, copy.root())
    };
    let before = dir_state(dir);
    let mut failures = snap::check(mode, snapshots, &ids, &run);
    // And the run writes no file under tests/snapshots/
    if mode == snap::Mode::Verify {
        match (before, dir_state(dir)) {
            (Ok(before), Ok(after)) if before == after => {},
            (Ok(_), Ok(_)) => failures.push(format!("a verifying run changed {}", dir.display())),
            (Err(problem), _) | (_, Err(problem)) => failures.push(problem),
        }
    }
    failures
}

#[test]
fn an_unreadable_snapshot_dir_fails_the_unchanged_check() {
    let not_a_dir = tempfile::NamedTempFile::new().expect("a file where the directory goes");
    assert!(dir_state(not_a_dir.path()).is_err());
    let missing = not_a_dir.path().with_extension("absent");
    assert_eq!(dir_state(&missing), Ok(Vec::new()));
}

/// A golden file's name and its bytes, or the error reading it.
type GoldenFile = (String, Result<Vec<u8>, String>);

/// Every golden file in `dir`, pending ones included, with its bytes or the
/// error reading it. Other snapshot tests' pending files are theirs to report.
/// A directory that cannot be read is an error, so the check before and after
/// a run cannot pass without having read anything.
fn dir_state(dir: &std::path::Path) -> Result<Vec<GoldenFile>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    let mut files: Vec<GoldenFile> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("golden__"))
        .map(|entry| {
            let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string());
            (entry.file_name().to_string_lossy().into_owned(), bytes)
        })
        .collect();
    files.sort();
    Ok(files)
}

#[test]
fn an_unreadable_golden_is_named_by_the_verifier_not_a_panic() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir(dir.path().join("golden__p_human.snap"))
        .expect("a directory where the file goes");
    let case = cases::all()
        .into_iter()
        .find(|case| case.id == "p_human")
        .expect("p_human");
    let failures = verify_all(
        snap::Mode::Verify,
        &snap::Snapshots::at(dir.path().into()),
        &[case],
    );
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("p_human: cannot read"),
        "{failures:?}"
    );
}

#[test]
fn the_harnesss_case_list_equals_this_specs_cases_table() {
    // Given the Cases table in specs/44-golden-outputs.md
    let spec =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(table::SPEC))
            .expect("the spec");
    // When the golden test reads it
    let rows = table::spec_rows(&spec);
    let cases = cases::all();
    let harness: Vec<(&str, i32)> = cases.iter().map(|case| (case.id, case.exit)).collect();
    // Then the harness's case IDs equal the table's IDs, with none missing and none extra
    // And every case's recorded exit status equals the table's Exit column
    assert_eq!(rows.len(), 131, "the Cases table");
    assert_eq!(table::compare(&rows, &harness), Vec::<String>::new());
    // And deleting a case from the harness, together with its snapshot, fails the test
    let without_one = &harness[1..];
    assert_eq!(
        table::compare(&rows, without_one),
        [format!(
            "{}: in the spec, missing from the harness",
            harness[0].0
        )]
    );
}

#[test]
fn no_case_enables_duplicate_triage() {
    // Given `just test` runs the tests with default features and with --all-features
    // (config::just_test_and_ci_verify_without_writing pins both runs), and the
    // golden test runs under each against the same committed snapshots
    // Then no case enables duplicate triage, the one output the triage build adds
    let configs = run::fixtures().join("golden/configs");
    for case in cases::all() {
        if let Some(name) = case.config {
            let config = std::fs::read_to_string(configs.join(name)).expect("a config");
            assert!(
                !config.contains("triage"),
                "{}: {name} configures triage",
                case.id
            );
        }
        assert!(
            case.env.iter().all(|(key, _)| !key.contains("TYPESAFE")),
            "{} sets a triage key",
            case.id
        );
    }
}

#[test]
fn no_insta_update_value_writes_under_tests_snapshots() {
    // A recording run changes goldens on purpose, and its own test reports them.
    if std::env::var_os("GOLDEN_RECORD").is_some() {
        return;
    }
    let before = dir_state(&snapshots_dir()).expect("tests/snapshots");
    for value in ["always", "force", "unseen", "new", "auto", "no"] {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "every_case_matches_the_golden_recorded_from_main_at_4b57930",
                "--exact",
            ])
            .env_remove("GOLDEN_RECORD")
            .env("INSTA_UPDATE", value)
            .output()
            .expect("re-running the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "INSTA_UPDATE={value}: {stdout}"
        );
        assert!(output.status.success(), "INSTA_UPDATE={value}: {stdout}");
        assert_eq!(
            dir_state(&snapshots_dir()).expect("tests/snapshots"),
            before,
            "INSTA_UPDATE={value} changed tests/snapshots/"
        );
    }
}

#[test]
fn equal_scores_in_one_file_keep_source_order_under_the_default_sort() {
    // Given the group fixture, whose three functions share one file and one score
    let source = std::fs::read_to_string(run::fixtures().join("golden/dup_group/src/lib.rs"))
        .expect("the group fixture");
    let declared: Vec<&str> = source
        .lines()
        .filter_map(|l| l.strip_prefix("pub fn "))
        .filter_map(|l| l.split('(').next())
        .collect();
    assert_eq!(declared.len(), 3);
    let mut alphabetical = declared.clone();
    alphabetical.sort_unstable();
    assert_ne!(
        declared, alphabetical,
        "the fixture must not declare in name order"
    );

    // When case g_group_human runs with the default --sort crap
    let case = cases::all()
        .into_iter()
        .find(|case| case.id == "g_group_human")
        .expect("the g_group_human case");
    assert!(!case.args.contains(&"--sort"));
    let (run, copy) = run::run(&case).expect("a run");
    let run = checks::finish(&case, run, copy.root()).expect("the output checks");
    let stdout = String::from_utf8(run.stdout).expect("utf-8");

    // Then the rows appear in source order, all at one score
    let rows: Vec<(&str, &str)> = stdout
        .lines()
        .filter(|l| l.starts_with('│') && l.contains("./src/lib.rs"))
        .map(|l| {
            let cells: Vec<&str> = l.split('┆').map(str::trim).collect();
            (cells[1], cells[4])
        })
        .collect();
    assert_eq!(
        rows.iter().map(|(_, f)| *f).collect::<Vec<_>>(),
        declared,
        "{stdout}"
    );
    assert!(
        rows.iter().all(|(score, _)| *score == rows[0].0),
        "{stdout}"
    );
}

#[test]
fn a_cases_bytes_do_not_depend_on_file_creation_order() {
    // Given a fixture with more than one source file
    let cases: Vec<run::Case> = cases::all()
        .into_iter()
        .filter(|case| matches!(case.fixture, run::Fixture::W | run::Fixture::D))
        .collect();
    let count = |fixture| cases.iter().filter(|case| case.fixture == fixture).count();
    assert_eq!((count(run::Fixture::W), count(run::Fixture::D)), (23, 17));
    // ext4 with dir_index, the CI runner's /tmp, lists a directory in hash
    // order whatever order its files were created in. tmpfs lists them in
    // creation order, so the copies go to /dev/shm where there is one.
    let shm = std::path::Path::new("/dev/shm");
    let parent = if shm.is_dir() {
        shm.to_path_buf()
    } else {
        std::env::temp_dir()
    };
    let mut reordered = false;
    for case in &cases {
        // When a case runs on one copy created in directory-listing order
        let (forward, forward_files) = run::prepare_ordered(case, &parent, run::Order::Forward);
        // And on a second copy whose files were created in reverse order
        let (reverse, reverse_files) = run::prepare_ordered(case, &parent, run::Order::Reverse);
        let mut reversed = forward_files.clone();
        reversed.reverse();
        assert_eq!(
            reverse_files, reversed,
            "{}: the copies were not built in opposite orders",
            case.id
        );
        reordered |= listings(forward.root()) != listings(reverse.root());
        // Then both runs give the same bytes
        let document = |copy: &run::CaseDir| {
            let run =
                run::run_on(case, copy).and_then(|run| checks::finish(case, run, copy.root()));
            encode::encode(
                case.id,
                &run.unwrap_or_else(|e| panic!("{}: {e:?}", case.id)),
            )
        };
        assert_eq!(
            document(&forward),
            document(&reverse),
            "{} depends on creation order",
            case.id
        );
    }
    if !reordered {
        let filesystem = filesystem_of(&parent);
        // CI runs on Linux, where this check is the only place the
        // scenario can fail: a filesystem there that ignores creation
        // order must not pass silently.
        assert!(
            std::env::consts::OS != "linux",
            "{filesystem} lists both copies identically: the creation-order check proved nothing"
        );
        eprintln!(
            "SKIPPED: {filesystem} lists every directory of both copies identically, so \
             a_cases_bytes_do_not_depend_on_file_creation_order proved nothing here"
        );
    }
}

/// Every directory under `root`, with its entries in the order `read_dir` lists them.
fn listings(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<std::ffi::OsString>)> {
    let mut all = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("a copy's directory") {
            let entry = entry.expect("an entry");
            if entry.file_type().expect("a file type").is_dir() {
                pending.push(entry.path());
            }
            names.push(entry.file_name());
        }
        let relative = dir
            .strip_prefix(root)
            .expect("inside the copy")
            .to_path_buf();
        all.push((relative, names));
    }
    all.sort();
    all
}

/// The filesystem type holding `path`, as `stat` (Linux) or `mount`
/// (macOS) names it.
fn filesystem_of(path: &std::path::Path) -> String {
    let output = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let shown = path.display().to_string();
    if let Some(kind) = output("stat", &["-f", "-c", "%T", &shown]) {
        return format!("the {kind} filesystem holding {shown}");
    }
    let mount_point = output("df", &["-P", &shown]).and_then(|df| {
        df.lines()
            .last()
            .and_then(|l| l.split_whitespace().last())
            .map(str::to_owned)
    });
    let kind = mount_point.as_ref().and_then(|point| {
        output("mount", &[])?
            .lines()
            .find(|line| line.contains(&format!(" on {point} (")))
            .and_then(|line| line.split(" (").nth(1))
            .and_then(|rest| rest.split([',', ')']).next())
            .map(str::to_owned)
    });
    format!(
        "the {} filesystem holding {shown}",
        kind.unwrap_or_else(|| "unknown".into())
    )
}
