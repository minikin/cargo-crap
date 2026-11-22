use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn binary(cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crapts"));
    cmd.current_dir(cwd);
    for key in [
        "GITHUB_REPOSITORY",
        "GITHUB_SERVER_URL",
        "GITHUB_SHA",
        "NO_COLOR",
        "FORCE_COLOR",
        "COLUMNS",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

fn invoke(
    cwd: &Path,
    args: &[&str],
) -> Output {
    binary(cwd).args(args).output().unwrap()
}

fn check(
    out: &Output,
    code: i32,
) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn settings(
    cwd: &Path,
    args: &[&str],
) -> Value {
    let out = binary(cwd)
        .arg("--print-config")
        .args(args)
        .output()
        .unwrap();
    check(&out, 0);
    assert_eq!(out.stderr.as_slice(), b"");
    assert!(out.stdout.ends_with(b"\n"));
    assert!(out.stdout.starts_with(b"{\n  \""));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn rejected(
    cwd: &Path,
    args: &[&str],
    detail: &str,
) {
    let out = invoke(cwd, args);
    check(&out, 2);
    assert!(out.stdout.is_empty(), "{args:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(detail), "{args:?}: {err}");
}

#[test]
fn help_and_version_have_priority_over_scoring_and_config() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".crapts.toml"), "[").unwrap();
    for help in ["-h", "--help", "-hV"] {
        let o = invoke(dir.path(), &["--bogus", "--print-config", "-V", help]);
        check(&o, 0);
        assert_eq!(o.stderr.as_slice(), b"");
        assert!(String::from_utf8_lossy(&o.stdout).contains("Usage: crapts"));
    }
    let o = invoke(dir.path(), &["--bogus", "--print-config", "-V"]);
    check(&o, 0);
    assert_eq!(o.stderr.as_slice(), b"");
    assert_eq!(
        o.stdout,
        format!("crapts {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

#[test]
fn priority_options_respect_value_positions_and_the_option_terminator() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("hits.data"), "{}").unwrap();
    for (args, detail) in [
        (vec!["--coverage", "--help"], "ambiguous"),
        (
            vec!["--coverage", "hits.data", "--", "--help"],
            "path does not exist: --help",
        ),
        (vec!["--help=1"], "--help does not take an argument"),
        (
            vec!["--print-config=1"],
            "--print-config does not take an argument",
        ),
    ] {
        rejected(dir.path(), &args, detail);
    }
}

#[test]
fn print_config_resolves_settings_without_reading_scoring_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".crapts.toml");
    fs::write(
        &path,
        "threshold=15\ntop=3\nexclude=['gen/**']\nfail-regression=true",
    )
    .unwrap();
    let config = settings(
        dir.path(),
        &[
            "absent.ts",
            "--exclude",
            "cache/**",
            "--all-rows",
            "--output",
            "absent-dir/report",
        ],
    );
    assert_eq!(config["config_path"], json!(path.canonicalize().unwrap()));
    let s = &config["settings"];
    assert_eq!(s["threshold"]["value"].as_f64(), Some(15.0));
    assert_eq!(s["threshold"]["origin"], "config");
    assert_eq!(s["top"], json!({"value":null,"origin":"config"}));
    assert_eq!(s["min"], json!({"value":null,"origin":"default"}));
    assert_eq!(
        s["missing"],
        json!({"value":"pessimistic","origin":"default"})
    );
    assert_eq!(
        s["fail_regression"],
        json!({"value":true,"origin":"config"})
    );
    assert_eq!(
        s["exclude"],
        json!({"value":[{"pattern":"gen/**","origin":"config"},{"pattern":"cache/**","origin":"flag"}],"origin":"flag"})
    );
}

#[test]
fn print_config_records_explicit_equal_values_and_the_source_of_cleared_limits() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".crapts.toml"), "threshold=15\ntop=3").unwrap();
    let s = &settings(
        dir.path(),
        &["--threshold", "15", "--min", "6", "--all-rows"],
    )["settings"];
    assert_eq!(s["threshold"]["value"].as_f64(), Some(15.0));
    assert_eq!(s["threshold"]["origin"], "flag");
    assert_eq!(s["top"], json!({"value":null,"origin":"config"}));
    assert_eq!(s["min"], json!({"value":null,"origin":"flag"}));
    assert!(s.get("default_excludes").is_some() && s.get("fail_above").is_some());
}

#[test]
fn print_config_validates_the_settings_it_explains() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        settings(dir.path(), &["--fail-changed"])["settings"]["fail_changed"],
        json!({"value":true,"origin":"flag"})
    );
    for (args, detail) in [
        (vec!["--print-config", "--bogus"], "--bogus"),
        (vec!["--print-config", "--top", "0"], "0"),
        (vec!["--print-config", "--all-rows", "--top", "0"], "--top"),
        (vec!["--print-config", "--allow", "{,x}"], "{,x}"),
    ] {
        rejected(dir.path(), &args, detail);
    }
}

#[test]
fn the_nearest_config_wins_within_the_git_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let packages = repo.join("packages");
    let cwd = packages.join("app");
    fs::create_dir_all(&cwd).unwrap();
    assert!(
        Command::new("git")
            .current_dir(&repo)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    for (base, value) in [(dir.path(), 99), (repo.as_path(), 20)] {
        fs::write(base.join(".crapts.toml"), format!("threshold={value}")).unwrap();
    }
    let config = settings(&cwd, &[]);
    assert_eq!(
        config["config_path"],
        json!(repo.join(".crapts.toml").canonicalize().unwrap())
    );
    assert_eq!(
        config["settings"]["threshold"]["value"].as_f64(),
        Some(20.0)
    );
    fs::write(packages.join(".crapts.toml"), "top=3").unwrap();
    let config = settings(&cwd, &[]);
    assert_eq!(
        config["config_path"],
        json!(packages.join(".crapts.toml").canonicalize().unwrap())
    );
    assert_eq!(
        config["settings"]["top"],
        json!({"value":3,"origin":"config"})
    );
    assert_eq!(
        config["settings"]["threshold"]["value"].as_f64(),
        Some(30.0)
    );
    for base in [&packages, &repo] {
        fs::remove_file(base.join(".crapts.toml")).unwrap();
    }
    assert!(settings(&cwd, &[])["config_path"].is_null());
    fs::rename(repo.join(".git"), dir.path().join("metadata")).unwrap();
    fs::write(repo.join(".git"), "gitdir: ../metadata\n").unwrap();
    assert!(settings(&cwd, &[])["config_path"].is_null());
}

#[test]
fn outside_git_the_walk_reaches_the_filesystem_root() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("a/b");
    fs::create_dir_all(&cwd).unwrap();
    let path = dir.path().join(".crapts.toml");
    fs::write(&path, "threshold=17").unwrap();
    for no_git in [false, true] {
        let mut command = binary(&cwd);
        if no_git {
            command.env("PATH", dir.path().join("no-git"));
        }
        let out = command.arg("--print-config").output().unwrap();
        check(&out, 0);
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["config_path"], json!(path.canonicalize().unwrap()));
        assert_eq!(v["settings"]["threshold"]["value"].as_f64(), Some(17.0));
        assert_eq!(v["settings"]["threshold"]["origin"], "config");
    }
}

#[test]
fn config_reports_type_spelling_and_syntax_errors_at_their_source() {
    let dir = tempfile::tempdir().unwrap();
    for (source, detail) in [
        ("thresold=1", "unknown key thresold"),
        ("allow=[3]", "allow"),
        ("allow=[", "line 1, column"),
        (
            "uncovered-hints=true\nuncovered_hints=false",
            "duplicate uncovered-hints",
        ),
    ] {
        fs::write(dir.path().join(".crapts.toml"), source).unwrap();
        for args in [
            &["--print-config"][..],
            &["--coverage", "absent", "--format", "json"],
        ] {
            rejected(dir.path(), args, detail);
            rejected(dir.path(), args, ".crapts.toml");
        }
    }
}

#[test]
fn print_config_shows_every_flag_and_env_link_fallback_without_opening_the_sink() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(".crapts.toml"),"default_excludes=['vendor/**']\nexclude=['a/**']\nallow=['file/a.ts']\nuncovered_hints=true").unwrap();
    let args = [
        "--print-config",
        "first",
        "second",
        "--coverage",
        "gone.dat",
        "--threshold",
        "-20",
        "--fail-above",
        "--baseline",
        "no.json",
        "--fail-regression",
        "--epsilon",
        "0",
        "--show-unchanged",
        "--base",
        "HEAD",
        "--fail-changed",
        "--missing",
        "optimistic",
        "--min",
        "0",
        "--top",
        "9007199254740991",
        "--sort",
        "file",
        "--summary",
        "--format",
        "human",
        "--output",
        "out.txt",
        "--commit-ref",
        "override",
        "--comment-marker",
        "custom",
    ];
    let out = binary(d.path())
        .args(args)
        .env("GITHUB_SERVER_URL", "https://git.example")
        .env("GITHUB_REPOSITORY", "owner/repo")
        .env("GITHUB_SHA", "envsha")
        .output()
        .unwrap();
    check(&out, 0);
    assert_eq!(out.stderr.as_slice(), b"");
    assert!(!d.path().join("out.txt").exists());
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = &doc["settings"];
    for (k, v) in [
        ("paths", json!(["first", "second"])),
        ("coverage", json!("gone.dat")),
        ("threshold", json!(-20.0)),
        ("fail_above", json!(true)),
        ("baseline", json!("no.json")),
        ("fail_regression", json!(true)),
        ("epsilon", json!(0.0)),
        ("show_unchanged", json!(true)),
        ("base", json!("HEAD")),
        ("fail_changed", json!(true)),
        ("missing", json!("optimistic")),
        ("min", json!(0.0)),
        ("top", json!(9_007_199_254_740_991_u64)),
        ("sort", json!("file")),
        ("summary", json!(true)),
        ("format", json!("text")),
        ("output", json!("out.txt")),
        ("commit_ref", json!("override")),
        ("comment_marker", json!("custom")),
    ] {
        assert_eq!(s[k], json!({"value":v,"origin":"flag"}), "{k}");
    }
    assert_eq!(
        s["repo_url"],
        json!({"value":"https://git.example/owner/repo","origin":"default"})
    );
    assert_eq!(
        s["uncovered_hints"],
        json!({"value":true,"origin":"config"})
    );
    for (key, pattern) in [
        ("default_excludes", "vendor/**"),
        ("exclude", "a/**"),
        ("allow", "file/a.ts"),
    ] {
        assert_eq!(
            s[key],
            json!({"value":[{"pattern":pattern,"origin":"config"}],"origin":"config"})
        );
    }
}

#[test]
fn cli_numeric_boundaries_and_attached_values_are_checked_before_printing() {
    let d = tempfile::tempdir().unwrap();
    for (arg, v) in [
        ("--threshold=.5", 0.5),
        ("--threshold=+1.e2", 100.0),
        ("--threshold=-0.5e-1", -0.05),
    ] {
        assert_eq!(
            settings(d.path(), &[arg])["settings"]["threshold"]["value"].as_f64(),
            Some(v)
        );
    }
    for arg in [
        "--top=+1",
        "--top=01.0",
        "--top=9007199254740992",
        "--min=0x10",
        "--epsilon=inf",
        "--threshold=NaN",
        "--epsilon=-1",
    ] {
        rejected(
            d.path(),
            &["--print-config", arg],
            arg.split('=').next().unwrap(),
        );
    }
    for key in [
        "--fail-above",
        "--summary",
        "--all-rows",
        "--no-default-excludes",
        "--version",
    ] {
        rejected(
            d.path(),
            &["--print-config", &format!("{key}=1")],
            "does not take an argument",
        );
    }
}

#[test]
fn priority_tokens_inside_values_and_after_the_terminator_remain_data() {
    let d = tempfile::tempdir().unwrap();
    let s = settings(
        d.path(),
        &[
            "--coverage=--help",
            "--comment-marker=-V",
            "--",
            "--version",
            "--print-config",
            "-hV",
        ],
    );
    assert_eq!(s["settings"]["coverage"]["value"], "--help");
    assert_eq!(s["settings"]["comment_marker"]["value"], "-V");
    assert_eq!(
        s["settings"]["paths"]["value"],
        json!(["--version", "--print-config", "-hV"])
    );
    for opt in ["--help", "-h", "-Vh", "-hV"] {
        let o = invoke(d.path(), &["--threshold=oops", "-V", opt]);
        check(&o, 0);
        assert!(String::from_utf8_lossy(&o.stdout).contains("Usage: crapts"));
    }
    let o = invoke(d.path(), &["--coverage", "--help", "--version"]);
    check(&o, 0);
    assert_eq!(
        o.stdout,
        format!("crapts {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

#[test]
fn literal_hyphens_and_invalid_flag_globs_keep_their_meaning_with_a_config() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(".crapts.toml"), "allow=['good']").unwrap();
    let s = settings(d.path(), &["-", "--coverage", "-", "--top", "1"]);
    assert_eq!(s["settings"]["paths"]["value"], json!(["-"]));
    assert_eq!(s["settings"]["coverage"]["value"], "-");
    for value in ["", "multi\rline", "multi\nline"] {
        rejected(
            d.path(),
            &["--print-config", "--comment-marker", value],
            "--comment-marker",
        );
    }
    let out = invoke(d.path(), &["--print-config", "--allow", "bad/["]);
    check(&out, 2);
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("bad/[") && !err.contains(".crapts.toml"),
        "{err}"
    );
    rejected(
        d.path(),
        &["--print-config", "--threshold", "--sort", "file"],
        "ambiguous argument for --threshold",
    );
    rejected(
        d.path(),
        &["--print-config", "--coverage"],
        "missing value for --coverage",
    );
}

#[cfg(unix)]
#[test]
fn an_unrepresentable_print_config_path_fails_before_the_json_starts() {
    use std::os::unix::ffi::OsStringExt;
    let d = tempfile::tempdir().unwrap();
    let out = binary(d.path())
        .arg("--print-config")
        .arg(std::ffi::OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    check(&out, 2);
    assert_eq!(out.stdout.as_slice(), b"");
    assert!(String::from_utf8_lossy(&out.stderr).contains("UTF-8"));
}

#[test]
fn unknown_switches_name_the_option_with_or_without_an_attached_value() {
    let d = tempfile::tempdir().unwrap();
    for flag in ["--bogus", "--bogus=1"] {
        let output = invoke(d.path(), &["--print-config", flag]);
        check(&output, 2);
        assert_eq!(output.stdout.as_slice(), b"");
        assert_eq!(output.stderr.as_slice(), b"error: unknown option --bogus\n");
    }
}

use std::fmt::Write as _;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/score.ts"),"export function hot(a: boolean, b: boolean) { if (a) return 1; if (b) return 2; return 0; }\nexport function edge(a: boolean) { if (a) return 1; return 0; }\nexport function cold() { return 1; }\n").unwrap();
    fs::write(
        dir.path().join("hits.data"),
        "SF:src/score.ts\nDA:1,0\nDA:2,0\nDA:3,1\nend_of_record\n",
    )
    .unwrap();
    dir
}

fn score(
    cwd: &Path,
    args: &[&str],
) -> Output {
    binary(cwd)
        .args(["--coverage", "hits.data", "--threshold", "6"])
        .args(args)
        .output()
        .unwrap()
}

fn json_score(
    cwd: &Path,
    args: &[&str],
    code: i32,
) -> Value {
    let o = binary(cwd)
        .args([
            "--coverage",
            "hits.data",
            "--format",
            "json",
            "--threshold",
            "6",
        ])
        .args(args)
        .output()
        .unwrap();
    check(&o, code);
    serde_json::from_slice(&o.stdout).unwrap()
}

fn names(doc: &Value) -> Vec<&str> {
    doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["function"].as_str().unwrap())
        .collect()
}

#[test]
fn an_absolute_report_identifies_the_tool_inputs_and_scores() {
    let dir = fixture();
    let out = score(dir.path(), &[".", "src/score.ts", "--format", "json"]);
    check(&out, 0);
    assert_eq!(out.stderr.as_slice(), b"");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/crapts/report-v1.json")).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&v)
        .unwrap();
    assert_eq!(v["$schema"], schema["$id"]);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["name_scheme"], 1);
    assert_eq!(v["coverage_kind"], "lines");
    assert_eq!(v["threshold"].as_f64(), Some(6.0));
    assert_eq!(v["selection"], Value::Null);
    assert_eq!(v["notices"], json!([]));
    assert_eq!(v["diagnostics"]["matched_files"], 1);
    assert_eq!(v["summary"]["total"], 3);
    assert_eq!(v["summary"]["crappy"], 1);
    for field in [
        "regressed",
        "improved",
        "new",
        "moved",
        "unchanged",
        "removed",
        "changed_failures",
    ] {
        assert_eq!(v["summary"].get(field), Some(&Value::Null), "{field}");
    }
    for field in ["removed", "regression_gate", "changed"] {
        assert!(v.get(field).is_none(), "{field}");
    }
    assert_eq!(names(&v), ["hot", "edge", "cold"]);
    for ((i, row), (cc, cov, crap)) in v["entries"].as_array().unwrap().iter().enumerate().zip([
        (3., 0., 12.),
        (2., 0., 6.),
        (1., 100., 1.),
    ]) {
        assert_eq!(row["file"], "src/score.ts");
        assert_eq!(row["line"], i + 1);
        assert_eq!(row["cyclomatic"].as_f64(), Some(cc));
        assert_eq!(row["coverage"].as_f64(), Some(cov));
        assert_eq!(row["crap"].as_f64(), Some(crap));
        assert!(row.get("fingerprint").is_none());
    }
}

#[test]
fn istanbul_input_is_recognized_under_an_lcov_filename() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f.ts"), "function f() { return 1; }").unwrap();
    let p = dir.path().join("hits.lcov");
    fs::write(&p,r#"{"f.ts":{"statementMap":{"0":{"start":{"line":1,"column":15},"end":{"line":1,"column":23}}},"s":{"0":1}}}"#).unwrap();
    let v = json_score(dir.path(), &["f.ts", "--coverage", "hits.lcov"], 0);
    assert_eq!(v["coverage_kind"], "statements");
    assert_eq!(v["entries"][0]["coverage"].as_f64(), Some(100.));
    assert_eq!(v["entries"][0]["crap"].as_f64(), Some(1.));
    fs::write(p, "{}").unwrap();
    fs::create_dir(dir.path().join("empty")).unwrap();
    let v = json_score(dir.path(), &["empty", "--coverage", "hits.lcov"], 0);
    assert_eq!(v["coverage_kind"], "statements");
    assert_eq!(v["summary"]["total"], 0);
    assert_eq!(v["summary"]["crappy"], 0);
    assert_eq!(v["entries"], json!([]));
    assert_eq!(v["diagnostics"]["matched_files"], 0);
}

#[test]
fn only_a_strictly_greater_score_trips_an_enabled_gate() {
    let d = fixture();
    let v = json_score(d.path(), &["--min", "13", "--fail-above"], 1);
    assert_eq!(v["summary"]["total"], 3);
    assert_eq!(v["summary"]["crappy"], 1);
    assert_eq!(names(&v), Vec::<&str>::new());
    assert_eq!(v["selection"]["min"].as_f64(), Some(13.));
    assert!(v["selection"]["top"].is_null());
    assert_eq!(v, json_score(d.path(), &["--min", "13"], 0));
    let v = json_score(
        d.path(),
        &["--min", "13", "--fail-above", "--allow", "hot"],
        0,
    );
    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["crappy"], 0);
}

#[test]
fn top_chooses_the_worst_rows_before_the_requested_display_sort() {
    let d = fixture();
    let v = json_score(d.path(), &["--min", "6", "--top", "2", "--sort", "file"], 0);
    assert_eq!(names(&v), ["edge", "hot"]);
    for args in [vec!["src", "."], vec![".", "src"]] {
        let mut flags = vec!["--min", "6", "--top", "1", "--sort", "file"];
        flags.extend(args);
        assert_eq!(names(&json_score(d.path(), &flags, 0)), ["hot"]);
    }
}

#[test]
fn all_rows_overrides_selection_without_changing_eligibility() {
    let d = fixture();
    fs::write(d.path().join(".crapts.toml"), "top=1\nallow=['edge']").unwrap();
    let v = json_score(
        d.path(),
        &[
            "--min",
            "13",
            "--sort",
            "file",
            "--fail-above",
            "--all-rows",
        ],
        1,
    );
    assert_eq!(names(&v), ["cold", "hot"]);
    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["crappy"], 1);
    assert!(v["selection"].is_null());
    assert_eq!(
        v,
        json_score(
            d.path(),
            &[
                "--all-rows",
                "--min",
                "13",
                "--sort",
                "file",
                "--fail-above"
            ],
            1
        )
    );
}

#[test]
fn repeated_scalar_options_use_the_last_finite_value() {
    let d = fixture();
    let v = json_score(
        d.path(),
        &[
            "--fail-above",
            "--threshold",
            "12",
            "--threshold=-1e1",
            "--min=-0.5",
        ],
        1,
    );
    assert_eq!(v["threshold"].as_f64(), Some(-10.));
    assert_eq!(v["selection"]["min"].as_f64(), Some(-0.5));
    assert_eq!(v["summary"]["crappy"], 3);
    let v = json_score(
        d.path(),
        &[
            "--fail-above",
            "--threshold=-1e1",
            "--threshold",
            "12",
            "--min=-0.5",
        ],
        0,
    );
    assert_eq!(v["threshold"].as_f64(), Some(12.));
    assert_eq!(v["summary"]["crappy"], 0);
}

#[test]
fn missing_coverage_follows_the_selected_policy_through_the_gate() {
    let d = fixture();
    fs::write(d.path().join("hits.data"), "{}").unwrap();
    for (policy, code, expected) in [("pessimistic", 1, 12.), ("optimistic", 0, 3.)] {
        let v = json_score(d.path(), &["--missing", policy, "--fail-above"], code);
        assert!(v["entries"][0]["coverage"].is_null());
        assert_eq!(v["entries"][0]["crap"].as_f64(), Some(expected));
        if code == 1 {
            assert_eq!(v, json_score(d.path(), &["--fail-above"], 1));
        }
    }
    let v = json_score(d.path(), &["--missing", "skip", "--fail-above"], 0);
    assert_eq!(names(&v), Vec::<&str>::new());
    assert_eq!(v["diagnostics"]["analyzed_files"], 1);
}

#[test]
fn config_gates_apply_according_to_the_selected_mode() {
    let d = fixture();
    fs::write(d.path().join(".crapts.toml"),"threshold=6\nfail-above=true\nfail-regression=true\nshow_unchanged=true\nfail-changed=true").unwrap();
    let cli = ["--coverage", "hits.data", "--format", "json"];
    let o = invoke(d.path(), &cli);
    check(&o, 1);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["summary"]["crappy"], 1);
    assert!(v.get("changed").is_none());
    assert_eq!(
        json_score(d.path(), &["--threshold", "12"], 0)["summary"]["crappy"],
        0
    );
    rejected(
        d.path(),
        &[
            "--coverage",
            "hits.data",
            "--threshold",
            "12",
            "--fail-changed",
        ],
        "--fail-changed needs --base",
    );
}

fn source(
    cwd: &Path,
    name: &str,
    content: &str,
) {
    let path = cwd.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn git_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap()
            .success()
    );
    repo
}

fn measured_files(
    cwd: &Path,
    files: &[(&str, &str)],
) {
    let mut lcov = String::new();
    for &(file, name) in files {
        source(cwd, file, &format!("function {name}() {{ return 0; }}"));
        writeln!(lcov, "SF:{file}\nDA:1,1\nend_of_record").unwrap();
    }
    fs::write(cwd.join("hits.data"), lcov).unwrap();
}

#[test]
fn file_and_flag_globs_keep_their_origins_across_a_subdirectory_run() {
    let repo = git_repo();
    let cwd = repo.path().join("packages/app");
    measured_files(
        &cwd,
        &[
            ("gen/a.ts", "a"),
            ("src/hot.ts", "hot"),
            ("src/edge.ts", "edge"),
            ("src/cold.ts", "cold"),
        ],
    );
    let path = repo.path().join(".crapts.toml");
    fs::write(
        &path,
        "exclude=['pack*/app/gen/**']\nallow=['pack*/app/src/hot.ts']",
    )
    .unwrap();
    let args = [
        "--format",
        "json",
        "--exclude",
        "src/edge.ts",
        "--allow",
        "cold",
    ];
    let o = score(&cwd, &args);
    check(&o, 0);
    assert_eq!(o.stderr.as_slice(), b"");
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(names(&v), Vec::<&str>::new());
    assert_eq!(v["diagnostics"]["analyzed_files"], 2);
    fs::remove_file(path).unwrap();
    let mut args = Vec::from(args);
    args.extend([
        "--exclude",
        "pack*/app/gen/**",
        "--allow",
        "pack*/app/src/hot.ts",
    ]);
    let o = score(&cwd, &args);
    check(&o, 0);
    assert_eq!(
        String::from_utf8(o.stderr).unwrap(),
        "warning: --exclude 'pack*/app/gen/**' excluded no file; globs are relative to the current directory\n"
    );
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(names(&v), ["a", "hot"]);
    assert_eq!(v["diagnostics"]["analyzed_files"], 3);
}

#[test]
fn a_config_path_allow_can_match_a_component_suffix() {
    let repo = git_repo();
    let cwd = repo.path().join("packages/app");
    measured_files(&cwd, &[("src/hot.ts", "hot"), ("lib/nothot.ts", "other")]);
    fs::write(repo.path().join(".crapts.toml"), "allow=['src/hot.ts']").unwrap();
    let v = json_score(&cwd, &[], 0);
    assert_eq!(names(&v), ["other"]);
    assert_eq!(v["diagnostics"]["matched_files"], 2);
}

#[test]
fn parent_config_defaults_replace_the_built_in_list_relative_to_the_config_directory() {
    let repo = git_repo();
    let cwd = repo.path().join("pkg");
    measured_files(
        &cwd,
        &[
            ("src/a.ts", "a"),
            ("src/a.test.ts", "test"),
            ("custom/b.ts", "b"),
        ],
    );
    let path = repo.path().join(".crapts.toml");
    fs::write(&path, "default_excludes=['pkg/custom/**']").unwrap();
    assert_eq!(
        names(&json_score(&cwd, &["--sort", "file"], 0)),
        ["test", "a"]
    );
    for flags in [vec!["--no-default-excludes"], vec![]] {
        if flags.is_empty() {
            fs::write(&path, "default-excludes=[]").unwrap();
        }
        assert_eq!(names(&json_score(&cwd, &flags, 0)).len(), 3);
        let mut flags = flags;
        flags.extend(["--exclude", "src/*.test.ts"]);
        assert_eq!(names(&json_score(&cwd, &flags, 0)), ["b", "a"]);
    }
}

#[test]
fn name_allows_use_the_grammar_of_the_emitted_names() {
    let dir = tempfile::tempdir().unwrap();
    source(
        dir.path(),
        "f.ts",
        "class Cart { add() { function inner() {} } add() {} } class Basket { add() {} }",
    );
    fs::write(
        dir.path().join("hits.data"),
        "SF:f.ts\nDA:1,1\nend_of_record",
    )
    .unwrap();
    assert_eq!(
        names(&json_score(
            dir.path(),
            &["--sort", "file", "--allow", "{Cart::add,Cart::add#*}"],
            0
        )),
        ["Basket::add", "Cart::add::inner"]
    );
    assert_eq!(
        names(&json_score(dir.path(), &["--allow", "Cart::*"], 0)),
        ["Basket::add"]
    );
    rejected(
        dir.path(),
        &["--allow", "{,x}", "--coverage", "hits.data"],
        "{,x}",
    );
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    fn project(decisions: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut text = String::new();
        let mut report = String::from("SF:rows.ts\n");
        for (i, k) in decisions.iter().enumerate() {
            writeln!(
                text,
                "function f{i}(a: boolean) {{ {}return 0; }}",
                "if (a) return 1; ".repeat(usize::from(*k))
            )
            .unwrap();
            writeln!(report, "DA:{},0", i + 1).unwrap();
        }
        report.push_str("end_of_record\n");
        source(dir.path(), "rows.ts", &text);
        fs::write(dir.path().join("hits.data"), report).unwrap();
        dir
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]
        #[test]
        fn the_exit_verdict_and_counts_are_invariant_under_presentation(
            decisions in prop::collection::vec(0u8..4,0..12),
            threshold in prop::sample::select(vec![2u32,6,12,20]),
            top in 1usize..8,
            min in 0u8..25,
            by_file in any::<bool>(),
            gate in any::<bool>(),
            summary in any::<bool>(),
            format in prop::sample::select(vec!["text","json","markdown","pr-comment","github","sarif","shields"])
        ) {
            let dir = project(&decisions);
            let threshold_text = threshold.to_string();
            let top = top.to_string();
            let min = min.to_string();
            let mut args = vec![
                "--threshold",
                &threshold_text,
                "--top",
                &top,
                "--min",
                &min,
                "--sort",
                if by_file { "file" } else { "crap" },
            ];
            if gate {
                args.push("--fail-above");
            }
            if summary {
                args.push("--summary");
            }
            let count = decisions
                .iter()
                .map(|&d| u32::from(d) + 1)
                .filter(|cc| cc * cc + cc > threshold)
                .count();
            let code = i32::from(gate && count > 0);
            let doc = json_score(dir.path(), &args, code);
            prop_assert_eq!(
                doc["summary"]["total"].as_u64(),
                Some(decisions.len() as u64)
            );
            prop_assert_eq!(doc["summary"]["crappy"].as_u64(), Some(count as u64));
            args.extend(["--format", format]);
            check(&score(dir.path(), &args), code);
        }

        #[test]
        fn all_rows_is_equivalent_to_clearing_the_two_limits(
            decisions in prop::collection::vec(0u8..3,0..12),
            top in 1usize..8,
            min in 0u8..15,
            by_file in any::<bool>(),
            allow in prop::sample::select(vec!["f1","f[34]","absent"]),
            exclude in any::<bool>(),
        ) {
            let dir = project(&decisions);
            let config = dir.path().join(".crapts.toml");
            let exclusions = if exclude { "rows.ts" } else { "none.ts" };
            let rules = format!("allow=['{allow}']\nexclude=['{exclusions}']");
            fs::write(&config, format!("{rules}\ntop={top}")).unwrap();
            let order = if by_file { "file" } else { "crap" };
            let min = min.to_string();
            let actual = score(
                dir.path(),
                &[
                    "--format",
                    "json",
                    "--all-rows",
                    "--min",
                    &min,
                    "--sort",
                    order,
                    "--fail-above",
                ],
            );
            fs::write(config, rules).unwrap();
            let expected = score(
                dir.path(),
                &["--format", "json", "--sort", order, "--fail-above"],
            );
            prop_assert!(matches!(actual.status.code(), Some(0 | 1)));
            prop_assert_eq!(actual.status.code(), expected.status.code());
            prop_assert_eq!(actual.stdout, expected.stdout);
        }
    }
}

#[test]
fn a_cut_through_tied_scores_uses_file_then_function_and_counts_all_violations() {
    let d = tempfile::tempdir().unwrap();
    for (file, text) in [
        (
            "b.ts",
            "function a(x,y) {if(x)return 1;if(y)return 2;return 0;}",
        ),
        (
            "a.ts",
            "function z(x,y) {if(x)return 1;if(y)return 2;return 0;}\nfunction a(x,y) {if(x)return 1;if(y)return 2;return 0;}",
        ),
    ] {
        source(d.path(), file, text);
    }
    source(
        d.path(),
        "hits.data",
        "SF:a.ts\nDA:1,0\nDA:2,0\nend_of_record\nSF:b.ts\nDA:1,0\nend_of_record\n",
    );
    for roots in [["b.ts", "a.ts"], ["a.ts", "b.ts"]] {
        let doc = json_score(
            d.path(),
            &[roots[0], roots[1], "--top", "1", "--fail-above"],
            1,
        );
        assert_eq!(doc["summary"]["crappy"], 3);
        assert_eq!(names(&doc), ["a"]);
        assert_eq!(doc["entries"][0]["file"], "a.ts");
        assert_eq!(doc["entries"][0]["line"], 2);
    }
}

#[cfg(unix)]
#[test]
fn config_and_scoring_share_one_worktree_probe() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fixture();
    let cwd = dir.path();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(cwd)
            .status()
            .unwrap()
            .success()
    );
    let tools = tempfile::tempdir().unwrap();
    let trace = tools.path().join("calls");
    let shim = tools.path().join("git");
    fs::write(&shim,"#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CRAPTS_GIT_TRACE\"\nexec \"$CRAPTS_REAL_GIT\" \"$@\"\n").unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let original = std::env::var_os("PATH").unwrap();
    let git = std::env::split_paths(&original)
        .map(|dir| dir.join("git"))
        .find(|file| file.is_file())
        .unwrap();
    let search = std::env::join_paths(
        std::iter::once(tools.path().to_owned()).chain(std::env::split_paths(&original)),
    )
    .unwrap();
    let output = binary(cwd)
        .args(["--coverage", "hits.data", "--format", "json"])
        .env("PATH", search)
        .env("CRAPTS_REAL_GIT", git)
        .env("CRAPTS_GIT_TRACE", &trace)
        .output()
        .unwrap();
    check(&output, 0);
    let calls = fs::read_to_string(trace).unwrap();
    assert_eq!(
        calls.matches("rev-parse --show-toplevel").count(),
        1,
        "{calls}"
    );
    assert_eq!(
        calls
            .matches("rev-parse --is-inside-work-tree --show-prefix")
            .count(),
        1,
        "{calls}"
    );
    assert_eq!(calls.matches("ls-files").count(), 1, "{calls}");
}

#[test]
fn the_absolute_report_schema_accepts_the_cli_signed_threshold() {
    let d = fixture();
    let doc = json_score(
        d.path(),
        &["--threshold=-1e1", "--min=-0.5", "--fail-above"],
        1,
    );
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/crapts/report-v1.json")).unwrap();
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&doc)
        .unwrap();
    assert_eq!(doc["threshold"].as_f64(), Some(-10.));
}
