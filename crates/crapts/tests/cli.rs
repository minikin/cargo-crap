mod common;
mod support;

use common::{binary, check, invoke, validated_report};

use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

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
        let help = String::from_utf8(o.stdout).unwrap();
        assert!(help.contains("Usage: crapts"));
        assert!(
            help.lines()
                .any(|line| line.trim_start().starts_with("--coverage <file>"))
        );
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
    let top = invoke(dir.path(), &["--print-config", "--top", "0"]);
    check(&top, 2);
    assert_eq!(top.stdout.as_slice(), b"");
    assert_eq!(
        top.stderr,
        b"error: --top must be an integer of at least 1, got 0\n"
    );
    for (args, detail) in [
        (vec!["--print-config", "--bogus"], "--bogus"),
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
    let path = dir.path().canonicalize().unwrap().join(".crapts.toml");
    for (source, detail) in [
        ("thresold=1", "unknown key thresold"),
        ("fail_above=true", "unknown key fail_above"),
        ("fail_regression=true", "unknown key fail_regression"),
        ("fail_changed=true", "unknown key fail_changed"),
        ("coverage='hits.data'", "unknown key coverage"),
        ("format='json'", "unknown key format"),
        ("output='out'", "unknown key output"),
        ("base='HEAD'", "unknown key base"),
        ("all-rows=true", "unknown key all-rows"),
        ("threshold='15'", "threshold"),
        ("fail-above=1", "fail-above"),
        ("top=0", "got 0"),
        ("epsilon=-1", "epsilon must be at least 0, got -1"),
        ("sort='size'", "size"),
        ("missing='maybe'", "maybe"),
        ("threshold=inf", "threshold must be finite"),
        ("allow=[3]", "allow"),
        ("allow=[", "line 1, column"),
        (
            "uncovered-hints=true\nuncovered_hints=false",
            "duplicate uncovered-hints",
        ),
    ] {
        let (key, _) = source.split_once('=').unwrap();
        fs::write(&path, source).unwrap();
        for args in [
            &["--print-config"][..],
            &["--coverage", "absent", "--format", "json"],
        ] {
            let out = invoke(dir.path(), args);
            check(&out, 2);
            assert_eq!(out.stdout.as_slice(), b"");
            let err = String::from_utf8(out.stderr).unwrap();
            assert!(
                err.contains(path.to_str().unwrap()) && err.contains(key) && err.contains(detail),
                "{source}: {err}"
            );
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
    let env_only = binary(d.path())
        .arg("--print-config")
        .env("GITHUB_SHA", "envsha")
        .output()
        .unwrap();
    check(&env_only, 0);
    let env_config: Value = serde_json::from_slice(&env_only.stdout).unwrap();
    assert_eq!(
        env_config["settings"]["commit_ref"],
        json!({"value": "envsha", "origin": "default"})
    );
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
    validated_report(&json_output(cwd, args, code).stdout)
}

fn json_output(
    cwd: &Path,
    args: &[&str],
    code: i32,
) -> Output {
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
    o
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
    let v = validated_report(&out.stdout);
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/crapts/report-v1.json")).unwrap();
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
        let fingerprint = row["fingerprint"].as_str().unwrap();
        assert_eq!(fingerprint.len(), 64);
        assert!(
            fingerprint
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
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
    assert_eq!(v["diagnostics"]["analyzed_files"], 0);
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
    assert_eq!(v["selection"], json!({"top": 2, "min": 6.0}));
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
    let out = json_output(
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
    let v = validated_report(&out.stdout);
    assert_eq!(names(&v), ["cold", "hot"]);
    assert_eq!(v["summary"]["total"], 2);
    assert_eq!(v["summary"]["crappy"], 1);
    assert!(v["selection"].is_null());
    assert_eq!(
        out.stdout,
        json_output(
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
        .stdout
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
        assert_eq!(v["entries"][0]["function"], "hot");
        assert!(v["entries"][0]["coverage"].is_null());
        assert_eq!(v["entries"][0]["crap"].as_f64(), Some(expected));
        if code == 1 {
            assert_eq!(v, json_score(d.path(), &["--fail-above"], 1));
        }
    }
    let v = json_score(d.path(), &["--missing", "skip", "--fail-above"], 0);
    assert_eq!(v["summary"]["total"], 0);
    assert_eq!(v["summary"]["crappy"], 0);
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
    assert_eq!(v["threshold"].as_f64(), Some(6.0));
    for count in [
        "new",
        "moved",
        "regressed",
        "improved",
        "unchanged",
        "removed",
        "changed_failures",
    ] {
        assert_eq!(v["summary"].get(count), Some(&Value::Null), "{count}");
    }
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
        names(&json_score(dir.path(), &["--sort", "file"], 0)),
        [
            "Basket::add",
            "Cart::add",
            "Cart::add#2",
            "Cart::add::inner"
        ]
    );
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
    fs::create_dir(dir.path().join("empty")).unwrap();
    rejected(
        dir.path(),
        &["empty", "--allow", "{,x}", "--coverage", "hits.data"],
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
        #![proptest_config(ProptestConfig { cases: 24, ..support::proptest_config() })]
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
    assert_eq!(doc["threshold"].as_f64(), Some(-10.));
}

fn rendered(
    cwd: &Path,
    format: &str,
    extra: &[&str],
    code: i32,
) -> String {
    let mut args = vec!["--format", format, "--fail-above"];
    args.extend_from_slice(extra);
    let out = score(cwd, &args);
    check(&out, code);
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

const MARKER: &str = "<!-- crapts-report -->";

#[test]
fn each_format_produces_a_usable_absolute_report() {
    let d = fixture();
    let run = |format| rendered(d.path(), format, &[], 1);
    let text = run("text");
    assert!(text.ends_with("\u{2717} 1/3 function(s) exceed CRAP threshold 6.\n"));
    assert_eq!(text, run("human"));
    assert_eq!(text.as_bytes(), score(d.path(), &["--fail-above"]).stdout);
    let md = run("markdown");
    let pr = run("pr-comment");
    assert_eq!(md.lines().next(), Some(MARKER));
    assert_eq!(pr.lines().next(), Some(MARKER));
    for name in ["hot", "edge", "cold"] {
        assert!(text.contains(name));
        assert!(md.contains(name));
    }
    assert!(
        pr.contains("hot")
            && pr.contains("3 function(s) analyzed")
            && !pr.contains("cold")
            && !pr.contains("edge")
    );
    assert_eq!(
        names(&serde_json::from_str(&run("json")).unwrap()),
        ["hot", "edge", "cold"]
    );
    let github = run("github");
    assert_eq!(github.lines().count(), 1);
    assert!(github.starts_with("::warning "));
    assert!(
        github.contains("file=src/score.ts") && github.contains("line=1") && github.contains("hot")
    );
    let sarif: Value = serde_json::from_str(&run("sarif")).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    let log = &sarif["runs"][0];
    assert_eq!(log["tool"]["driver"]["name"], "crapts");
    assert_eq!(log["results"].as_array().unwrap().len(), 1);
    let result = &log["results"][0];
    assert_eq!(result["ruleId"], "crap/high-score");
    assert_eq!(result["level"], "warning");
    let loc = &result["locations"][0]["physicalLocation"];
    assert_eq!(loc["artifactLocation"]["uri"], "src/score.ts");
    assert_eq!(loc["region"]["startLine"], 1);
    let badge: Value = serde_json::from_str(&run("shields")).unwrap();
    assert_eq!(
        badge,
        json!({"schemaVersion":1,"label":"CRAP > 6","message":"1 crappy","color":"yellow"})
    );
}

#[test]
fn a_passing_run_has_no_findings_in_any_format() {
    let d = fixture();
    let run = |f| rendered(d.path(), f, &["--threshold", "12"], 0);
    assert!(
        run("text").ends_with("\u{2713} 3 function(s) analyzed; none exceed CRAP threshold 12.\n")
    );
    for format in ["markdown", "pr-comment"] {
        assert_eq!(run(format).lines().next(), Some(MARKER));
    }
    let doc: Value = serde_json::from_str(&run("json")).unwrap();
    assert_eq!(doc["summary"]["crappy"], 0);
    assert_eq!(run("github"), "");
    let v: Value = serde_json::from_str(&run("sarif")).unwrap();
    assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "crapts");
    assert_eq!(v["runs"][0]["results"], json!([]));
    let badge: Value = serde_json::from_str(&run("shields")).unwrap();
    assert_eq!(badge["color"], "brightgreen");
    assert_eq!(badge["message"], "passing");
}

fn many_functions(
    dir: &Path,
    decisions: &[usize],
) {
    use std::fmt::Write;
    let mut text = String::new();
    let mut lcov = String::from("SF:many.ts\n");
    for (i, &n) in decisions.iter().enumerate() {
        writeln!(
            text,
            "function row{i:02}(x: boolean) {{ {}return 0; }}",
            "if(x) return 1; ".repeat(n)
        )
        .unwrap();
        writeln!(lcov, "DA:{},0", i + 1).unwrap();
    }
    source(dir, "many.ts", &text);
    fs::write(dir.join("hits.data"), lcov + "end_of_record\n").unwrap();
}

#[test]
fn the_badge_changes_colour_at_six_findings() {
    let d = tempfile::tempdir().unwrap();
    for (n, color) in [(1, "yellow"), (5, "yellow"), (6, "red")] {
        many_functions(d.path(), &vec![2; n]);
        let badge: Value = serde_json::from_str(&rendered(d.path(), "shields", &[], 1)).unwrap();
        assert_eq!(badge["label"], "CRAP > 6");
        assert_eq!(badge["color"], color);
        assert_eq!(badge["message"], format!("{n} crappy"));
    }
}

#[test]
fn an_empty_display_slice_still_runs_the_gate_on_every_eligible_row() {
    let d = fixture();
    let run = |f| rendered(d.path(), f, &["--min", "13"], 1);
    assert_eq!(run("text"), "No functions found.\n");
    assert_eq!(run("github"), "");
    let sarif: Value = serde_json::from_str(&run("sarif")).unwrap();
    assert_eq!(sarif["runs"][0]["results"], json!([]));
    assert_eq!(sarif["runs"][0]["tool"]["driver"]["name"], "crapts");
    assert_eq!(
        serde_json::from_str::<Value>(&run("shields")).unwrap(),
        json!({"schemaVersion":1,"color":"brightgreen","label":"CRAP > 6","message":"passing"})
    );
}

#[test]
fn the_human_renderer_reports_the_population_it_receives() {
    let d = fixture();
    assert!(
        rendered(d.path(), "text", &["--top", "1"], 1)
            .ends_with("\u{2717} 1/1 function(s) exceed CRAP threshold 6.\n")
    );
    fs::create_dir(d.path().join("empty")).unwrap();
    fs::write(d.path().join("hits.data"), "{}").unwrap();
    assert_eq!(
        rendered(d.path(), "text", &["empty"], 0),
        "No functions found.\n"
    );
}

#[test]
fn summary_respects_the_format_and_the_selected_rows() {
    let d = fixture();
    for f in ["text", "markdown", "pr-comment"] {
        let full = rendered(d.path(), f, &["--summary"], 1);
        let empty = rendered(d.path(), f, &["--summary", "--min", "13"], 1);
        assert!(
            full.contains("Analyzed: 3")
                && full.contains("Crappy: 1")
                && full.contains("Worst: hot (CRAP 12.0)"),
            "{full}"
        );
        assert!(
            empty.contains("Analyzed: 0")
                && empty.contains("Crappy: 0")
                && !empty.contains("Worst"),
            "{empty}"
        );
        let by_file = rendered(d.path(), f, &["--summary", "--sort", "file"], 1);
        assert!(by_file.contains("Worst: cold (CRAP 1.0)"), "{by_file}");
        if f != "text" {
            assert_eq!(full.lines().next(), Some(MARKER));
            assert_eq!(empty.lines().next(), Some(MARKER));
        }
    }
    for f in ["json", "github", "sarif", "shields"] {
        assert_eq!(
            rendered(d.path(), f, &["--summary", "--min", "13"], 1),
            rendered(d.path(), f, &["--min", "13"], 1)
        );
    }
}

fn link_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path().join("packages/app");
    source(
        &cwd,
        "src/score #1.ts",
        "function hot(a,b) { if(a) return 1; if(b) return 2; return 0; }",
    );
    fs::write(
        cwd.join("hits.data"),
        "SF:src/score #1.ts\nDA:1,0\nend_of_record\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap()
            .success()
    );
    (d, cwd)
}

fn link_output(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> String {
    let out = binary(cwd)
        .args(["--coverage", "hits.data", "--threshold", "6"])
        .args(args)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    check(&out, 0);
    assert_eq!(out.stderr.as_slice(), b"");
    String::from_utf8(out.stdout).unwrap()
}

const GITHUB_ENV: [(&str, &str); 3] = [
    ("GITHUB_SERVER_URL", "https://github.com"),
    ("GITHUB_REPOSITORY", "org/repo"),
    ("GITHUB_SHA", "envsha"),
];

#[test]
fn pr_comment_markers_and_links_belong_to_the_invocation() {
    let (_d, cwd) = link_fixture();
    let marker = "<!-- local -->";
    let mut args = vec![
        "--format",
        "pr-comment",
        "--commit-ref",
        "abc",
        "--comment-marker",
        marker,
    ];
    let full = link_output(&cwd, &args, &GITHUB_ENV);
    assert_eq!(
        full.matches("https://github.com/org/repo/blob/abc/packages/app/src/score%20%231.ts#L1")
            .count(),
        2
    );
    args.push("--summary");
    let summary = link_output(&cwd, &args, &GITHUB_ENV);
    for report in [full, summary] {
        assert_eq!(report.lines().next(), Some(marker));
        assert_eq!(report.matches(marker).count(), 1);
    }
    assert_eq!(
        link_output(&cwd, &["--format", "pr-comment"], &[])
            .lines()
            .next(),
        Some(MARKER)
    );
}

#[test]
fn repository_paths_are_projected_only_for_link_and_annotation_destinations() {
    let (_d, cwd) = link_fixture();
    let url = "https://github.com/org/repo/blob/envsha/packages/app/src/score%20%231.ts#L1";
    let md = link_output(
        &cwd,
        &["--format", "markdown", "--comment-marker", "local"],
        &GITHUB_ENV,
    );
    assert_eq!(md.matches(url).count(), 2);
    assert_eq!(md.lines().next(), Some(MARKER));
    let gh = link_output(&cwd, &["--format", "github"], &GITHUB_ENV);
    assert!(gh.contains("file=packages/app/src/score #1.ts") && gh.contains("line=1"));
    let sarif: Value =
        serde_json::from_str(&link_output(&cwd, &["--format", "sarif"], &GITHUB_ENV)).unwrap();
    let loc = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"];
    assert_eq!(
        loc["artifactLocation"]["uri"],
        "packages/app/src/score #1.ts"
    );
    assert_eq!(loc["region"]["startLine"], 1);
    assert_eq!(
        json_score(&cwd, &[], 0)["entries"][0]["file"],
        "src/score #1.ts"
    );
    for f in ["markdown", "pr-comment"] {
        for pair in [["--repo-url", "https://host"], ["--commit-ref", "abc"]] {
            let report = link_output(&cwd, &["--format", f, pair[0], pair[1]], &[]);
            assert!(!report.contains("]("));
            assert!(report.contains("hot"));
        }
    }
}

#[test]
fn worktree_boundaries_and_report_paths_when_git_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let cwd = repo.join("packages/app");
    measured_files(&cwd, &[("src/hot.ts", "hot")]);
    fs::write(dir.path().join(".crapts.toml"), "threshold=99").unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    let run = |args: &[&str]| {
        let out = binary(&cwd)
            .env("PATH", dir.path().join("no-git"))
            .args(args)
            .output()
            .unwrap();
        check(&out, 0);
        String::from_utf8(out.stdout).unwrap()
    };
    for gitfile in [false, true] {
        if gitfile {
            fs::remove_dir(repo.join(".git")).unwrap();
            fs::write(repo.join(".git"), "gitdir: ../gitdir\n").unwrap();
        }
        let config: Value = serde_json::from_str(&run(&["--print-config"])).unwrap();
        assert!(config["config_path"].is_null());
        assert_eq!(config["settings"]["threshold"]["value"].as_f64(), Some(30.));
        let md = run(&[
            "--coverage",
            "hits.data",
            "--format",
            "markdown",
            "--repo-url",
            "https://example.org/r",
            "--commit-ref",
            "abc",
        ]);
        assert!(md.contains("https://example.org/r/blob/abc/packages/app/src/hot.ts#L1"));
        let v: Value =
            serde_json::from_str(&run(&["--coverage", "hits.data", "--format", "json"])).unwrap();
        assert_eq!(v["entries"][0]["file"], "src/hot.ts");
    }
}

#[test]
fn workflow_commands_escape_properties_and_already_encoded_names() {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path().join("packages/app");
    source(
        &cwd,
        "src/a,b.ts",
        "class C { 'a/b'(a,b) { if(a) return 1; if(b) return 2; return 0; } }",
    );
    fs::write(
        cwd.join("hits.data"),
        "SF:src/a,b.ts\nDA:1,0\nend_of_record",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap()
            .success()
    );
    let out = rendered(&cwd, "github", &[], 1);
    assert!(
        out.contains("file=packages/app/src/a%2Cb.ts") && out.contains("a%252Fb"),
        "{out}"
    );
}

#[test]
fn a_pr_comment_caps_the_absolute_findings_table_at_25_rows() {
    let d = tempfile::tempdir().unwrap();
    let mut counts = vec![2; 27];
    counts.push(0);
    many_functions(d.path(), &counts);
    let pr = rendered(d.path(), "pr-comment", &["--comment-marker", "mine"], 1);
    assert_eq!(pr.lines().next(), Some("mine"));
    assert_eq!(pr.matches("row").count(), 25);
    assert!(!pr.contains("row27"));
    assert!(pr.contains("and 2 more") && pr.contains("28 function(s) analyzed"));
}

#[test]
fn a_marker_occupies_exactly_one_nonempty_line() {
    let d = fixture();
    for value in ["", "a\rb", "a\nb"] {
        rejected(
            d.path(),
            &[
                "--coverage",
                "hits.data",
                "--fail-above",
                "--format",
                "pr-comment",
                "--comment-marker",
                value,
            ],
            "--comment-marker",
        );
    }
}

#[test]
fn uncovered_cells_show_three_ranges_and_the_number_omitted() {
    use std::fmt::Write;
    let d = tempfile::tempdir().unwrap();
    source(
        d.path(),
        "f.ts",
        &format!("function f() {{\n{}return;\n}}", "work();\n".repeat(9)),
    );
    let mut lcov = String::from("SF:f.ts\n");
    for line in 1..=11 {
        writeln!(lcov, "DA:{line},{}", line % 2).unwrap();
    }
    fs::write(d.path().join("hits.data"), lcov + "end_of_record\n").unwrap();
    fs::write(d.path().join(".crapts.toml"), "uncovered_hints=true").unwrap();
    for f in ["text", "markdown", "pr-comment"] {
        let out = rendered(d.path(), f, &["--threshold", "0"], 1);
        assert!(out.contains("2, 4, 6") && out.contains("+2 more"), "{out}");
    }
}

#[test]
fn the_default_text_cap_applies_only_to_an_unsliced_report() {
    let d = tempfile::tempdir().unwrap();
    many_functions(d.path(), &(0..13).collect::<Vec<_>>());
    let args = ["--threshold", "160"];
    let text = rendered(d.path(), "text", &args, 1);
    assert!(text.contains("2 more below threshold"));
    assert!(
        !text.contains("row00")
            && !text.contains("row01")
            && text.contains("row02")
            && text.contains("row12")
    );
    for f in ["markdown", "text"] {
        let extra = if f == "text" {
            vec!["--threshold", "160", "--min", "0"]
        } else {
            args.to_vec()
        };
        let full = rendered(d.path(), f, &extra, 1);
        assert_eq!(full.matches("row").count(), 13);
    }
    let restored = ["--threshold", "160", "--min", "0", "--all-rows"];
    assert_eq!(text, rendered(d.path(), "text", &restored, 1));
    let doc = json_score(d.path(), &restored, 0);
    assert_eq!(names(&doc).len(), 13);
    assert!(doc["selection"].is_null());
}

fn plain_text(
    cwd: &Path,
    vars: &[(&str, &str)],
    args: &[&str],
) -> String {
    let out = binary(cwd)
        .args(["--coverage", "hits.data"])
        .args(args)
        .envs(vars.iter().copied())
        .output()
        .unwrap();
    check(&out, 0);
    String::from_utf8(out.stdout).unwrap()
}

#[cfg(unix)]
fn terminal_text(
    cwd: &Path,
    vars: &[(&str, &str)],
    args: &[&str],
) -> String {
    let mut command = binary(cwd);
    command
        .args(["--coverage", "hits.data"])
        .args(args)
        .envs(vars.iter().copied());
    run_in_pty(command)
}

#[cfg(unix)]
fn run_in_pty(mut command: Command) -> String {
    use std::{
        io::Read,
        os::fd::{FromRawFd, OwnedFd},
        process::Stdio,
    };
    let (mut reader, mut writer) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty initializes both descriptors on success. Neither optional output buffer is used.
    let result = unsafe {
        libc::openpty(
            &raw mut reader,
            &raw mut writer,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    // SAFETY: Each descriptor returned by openpty has exactly one owner here.
    let (mut master, slave) =
        unsafe { (fs::File::from_raw_fd(reader), OwnedFd::from_raw_fd(writer)) };
    let mut child = command.stdout(Stdio::from(slave)).spawn().unwrap();
    drop(command);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("{error}"),
        }
    }
    assert!(child.wait().unwrap().success());
    String::from_utf8(bytes).unwrap()
}

fn tier(
    text: &str,
    blocks: usize,
    cc: bool,
) {
    let longest = text
        .split(|c| c != '\u{2588}')
        .map(|run| run.chars().count())
        .max()
        .unwrap();
    assert_eq!(longest, blocks, "{text}");
    assert_eq!(text.contains(" CC "), cc, "{text}");
    assert!(text.contains("100.0%"), "{text}");
}

#[test]
fn the_text_width_selects_visible_columns() {
    let d = tempfile::tempdir().unwrap();
    measured_files(d.path(), &[("f.ts", "f")]);
    for (width, blocks, cc) in [
        ("100", 10, true),
        ("80", 5, true),
        ("60", 0, true),
        ("59", 0, false),
    ] {
        tier(
            &plain_text(d.path(), &[("COLUMNS", width)], &[]),
            blocks,
            cc,
        );
    }
    #[cfg(unix)]
    {
        tier(
            &terminal_text(d.path(), &[("COLUMNS", "59"), ("NO_COLOR", "1")], &[]),
            5,
            true,
        );
        terminal_text(d.path(), &[("COLUMNS", "59")], &["--output", "out.txt"]);
        tier(
            &fs::read_to_string(d.path().join("out.txt")).unwrap(),
            0,
            false,
        );
    }
}

#[test]
fn unicode_cells_are_cut_to_exact_display_columns() {
    use unicode_width::UnicodeWidthStr;
    let d = tempfile::tempdir().unwrap();
    measured_files(
        d.path(),
        &[(
            "deep/folder/\u{1f44d}.ts",
            "\u{6f22}\u{5b57}e\u{301}abcdefghijklmnop",
        )],
    );
    let table = plain_text(d.path(), &[("COLUMNS", "48"), ("NO_COLOR", "1")], &[]);
    assert!(
        table.contains("\u{6f22}\u{5b57}e\u{301}abcd\u{2026}")
            && table.contains("\u{2026}/\u{1f44d}.ts:1"),
        "{table}"
    );
    let lines = table
        .lines()
        .filter(|line| matches!(line.chars().next(), Some('\u{2500}'..='\u{257f}')))
        .collect::<Vec<_>>();
    assert!(lines.len() >= 4);
    for line in lines {
        assert_eq!(line.width(), 48, "{line}");
    }
}

#[test]
fn colour_follows_the_crapts_sink_and_environment() {
    let d = fixture();
    let ansi = |s: String| s.contains("\x1b[");
    for (vars, color) in [
        (vec![], false),
        (vec![("FORCE_COLOR", "1")], true),
        (vec![("FORCE_COLOR", "0")], false),
        (vec![("NO_COLOR", "1"), ("FORCE_COLOR", "1")], false),
    ] {
        assert_eq!(ansi(plain_text(d.path(), &vars, &[])), color);
    }
    assert_eq!(
        plain_text(d.path(), &[("FORCE_COLOR", "1")], &["--output", "out.txt"]),
        ""
    );
    assert!(!ansi(fs::read_to_string(d.path().join("out.txt")).unwrap()));
    #[cfg(unix)]
    for (vars, color) in [
        (vec![], true),
        (vec![("FORCE_COLOR", "0")], true),
        (vec![("NO_COLOR", "1"), ("FORCE_COLOR", "1")], false),
    ] {
        assert_eq!(ansi(terminal_text(d.path(), &vars, &[])), color);
    }
}

#[test]
fn github_link_defaults_need_nonempty_environment_components() {
    let (_repo, cwd) = link_fixture();
    for (server, repository, reference) in [
        ("", "owner/repo", "abc"),
        ("https://host", "", "abc"),
        ("https://host", "owner/repo", ""),
    ] {
        let env = [
            ("GITHUB_SERVER_URL", server),
            ("GITHUB_REPOSITORY", repository),
            ("GITHUB_SHA", reference),
        ];
        let report = link_output(&cwd, &["--format", "markdown"], &env);
        assert!(!report.contains("]("), "{report}");
    }
    for flag in ["--repo-url=", "--commit-ref="] {
        let report = link_output(&cwd, &["--format", "pr-comment", flag], &GITHUB_ENV);
        assert!(!report.contains("]("), "{report}");
    }
}

fn before_render(
    cwd: &Path,
    args: &[&str],
    causes: &[&str],
) -> String {
    before_render_as(cwd, args, causes, || binary(cwd))
}

fn before_render_as(
    cwd: &Path,
    args: &[&str],
    causes: &[&str],
    command: impl Fn() -> Command,
) -> String {
    let plain = command().args(args).output().unwrap();
    check(&plain, 2);
    assert_eq!(plain.stdout.as_slice(), b"");
    let diagnostic = String::from_utf8(plain.stderr).unwrap();
    for cause in causes {
        assert!(
            diagnostic.contains(cause),
            "{args:?} expected {cause}: {diagnostic}"
        );
    }
    let path = cwd.join("report.json");
    let mut with_output = vec!["--output", "report.json"];
    with_output.extend_from_slice(args);
    for existing in [false, true] {
        if existing {
            fs::write(&path, "keep").unwrap();
        }
        let out = command().args(&with_output).output().unwrap();
        check(&out, 2);
        assert_eq!(out.stdout.as_slice(), b"");
        assert_eq!(out.stderr, diagnostic.as_bytes());
        if existing {
            assert_eq!(fs::read(&path).unwrap(), b"keep");
            fs::remove_file(&path).unwrap();
        } else {
            assert!(!path.exists());
        }
    }
    diagnostic
}

#[test]
fn invalid_input_is_diagnosed_before_a_report_is_opened() {
    let d = fixture();
    before_render(d.path(), &[], &["--coverage <file> is required"]);
    for (flags, causes) in [
        (vec!["--bogus"], vec!["unknown option --bogus"]),
        (vec!["--coverage"], vec!["missing value for --coverage"]),
        (vec!["--format", "xml"], vec!["--format", "xml"]),
        (vec!["--sort", "size"], vec!["--sort", "size"]),
        (vec!["--missing", "maybe"], vec!["--missing", "maybe"]),
        (
            vec!["--top", "0", "--all-rows"],
            vec!["--top must be an integer of at least 1", "0"],
        ),
        (
            vec!["--top=1.5"],
            vec!["--top must be an integer of at least 1", "1.5"],
        ),
        (
            vec!["--top=1e1"],
            vec!["--top must be an integer of at least 1", "1e1"],
        ),
        (
            vec!["--top=9007199254740992"],
            vec!["--top must be an integer of at least 1", "9007199254740992"],
        ),
        (
            vec!["--threshold=NaN"],
            vec!["--threshold", "finite number"],
        ),
        (vec!["--min=0x10"], vec!["--min", "finite number"]),
        (vec!["--epsilon=1e999"], vec!["--epsilon", "finite number"]),
        (vec!["--epsilon=-0.5"], vec!["--epsilon must be at least 0"]),
        (
            vec!["missing-dir", "--exclude", "src/["],
            vec!["invalid glob", "src/["],
        ),
        (vec!["absent.ts"], vec!["path does not exist: absent.ts"]),
    ] {
        let mut args = vec!["--coverage", "hits.data"];
        args.extend(flags);
        before_render(d.path(), &args, &causes);
    }
}

#[test]
fn discovery_and_source_decoding_errors_exit_two_at_the_native_binary() {
    let d = fixture();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(d.path())
            .status()
            .unwrap()
            .success()
    );
    fs::write(d.path().join("invalid.ts"), [0x66, 0x80, 0x0a]).unwrap();
    let root = d.path().canonicalize().unwrap();
    for (file, cause) in [
        ("../outside.ts", root.to_str().unwrap()),
        ("invalid.ts", "invalid UTF-8"),
    ] {
        before_render(
            d.path(),
            &["--coverage", "hits.data", "--fail-above", file],
            &[file, cause],
        );
    }
}

fn coverage_error(
    cwd: &Path,
    file: &str,
    causes: &[&str],
) {
    let label = cwd.canonicalize().unwrap().join(file);
    let mut causes = causes.to_vec();
    causes.push(label.to_str().unwrap());
    before_render(
        cwd,
        &[
            "src/score.ts",
            "--coverage",
            file,
            "--format",
            "json",
            "--threshold",
            "6",
            "--fail-above",
        ],
        &causes,
    );
}

#[cfg(unix)]
#[test]
fn denied_coverage_reaches_stderr_before_opening_the_output() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    let d = fixture();
    let cwd = d.path();
    let path = cwd.join("denied");
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::File::open(&path).is_err() {
        coverage_error(cwd, "denied", &["EACCES"]);
        return;
    }

    let exe = cwd.join("runner");
    fs::copy(env!("CARGO_BIN_EXE_crapts"), &exe).unwrap();
    for path in [cwd.to_owned(), cwd.join("src"), exe.clone()] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let label = path.canonicalize().unwrap();
    before_render_as(
        cwd,
        &[
            "src/score.ts",
            "--coverage",
            "denied",
            "--format",
            "json",
            "--threshold",
            "6",
            "--fail-above",
        ],
        &["EACCES", label.to_str().unwrap()],
        || {
            let mut cmd = Command::new(&exe);
            cmd.current_dir(cwd).uid(65534).gid(65534);
            cmd
        },
    );
}

#[test]
fn coverage_input_errors_reach_stderr_before_the_native_cli_opens_a_report() {
    let d = fixture();
    let cwd = d.path();
    coverage_error(cwd, "gone.info", &["ENOENT"]);
    fs::create_dir(cwd.join("reports")).unwrap();
    coverage_error(cwd, "reports", &["EISDIR"]);
    fs::write(cwd.join("broken"), "{\nSF:a.ts").unwrap();
    coverage_error(cwd, "broken", &["invalid JSON"]);
    for input in ["", "[]", "\u{85}{}", "TN:prefix SF:x\nDA:1,1", "  SF:a.ts"] {
        fs::write(cwd.join("mystery"), input).unwrap();
        coverage_error(cwd, "mystery", &["unrecognized coverage format"]);
    }
    let run = |file: &str, input: String, detail: &[&str]| {
        fs::write(cwd.join(file), input).unwrap();
        coverage_error(cwd, file, detail);
    };
    for (input, detail) in [
        ("TN:x\nDA:8,1\nSF:a.ts", "line 2: DA outside an SF record"),
        (
            "TN:x\nDA:malformed\nSF:a.ts",
            "line 2: DA outside an SF record",
        ),
        (
            "SF:a.ts\nend_of_record\nDA:8,1",
            "line 3: DA outside an SF record",
        ),
        ("SF:a.ts\nSF:", "line 2: missing SF path"),
    ] {
        run("cov.info", input.to_owned(), &[detail]);
    }
    for (da, cause) in [
        ("0,1", "number"),
        ("9007199254740992,1", "number"),
        ("+1,+2", "record"),
        ("1,-1", "record"),
        ("1,1.5", "record"),
        ("1,1,", "record"),
        ("1,1,checksum,extra", "record"),
    ] {
        run(
            "cov.info",
            format!("SF:a.ts\nDA:{da}"),
            &[&format!("line 2: invalid DA {cause}")],
        );
    }
    let count = "0".repeat(308);
    run(
        "cov.info",
        format!("SF:a.ts\nDA:2,1{count}0"),
        &["line 2: invalid DA number"],
    );
    run(
        "cov.info",
        format!("SF:a.ts\nDA:2,1{count}\nDA:2,1{count}"),
        &["line 3: invalid DA number"],
    );
}

#[test]
fn istanbul_entry_and_statement_validation_precedes_output_creation() {
    let d = fixture();
    let file = d.path().join("run.json");
    let span = r#"{"start":{"line":2,"column":3},"end":{"line":2,"column":4}}"#;
    let run = |entry: &str, detail: &[&str]| {
        fs::write(&file, format!(r#"{{"src/a.ts":{entry}}}"#)).unwrap();
        let mut causes = vec!["src/a.ts"];
        causes.extend_from_slice(detail);
        coverage_error(d.path(), "run.json", &causes);
    };
    let entry = |spans: &str, hits: &str| format!(r#"{{"statementMap":{spans},"s":{hits}}}"#);
    let map = |value: &str| format!(r#"{{"x":{value}}}"#);
    for wrong in [
        "null",
        "[]",
        r#"{"lines":{"total":1}}"#,
        r#"{"s":{"x":0}}"#,
        r#"{"statementMap":[],"s":{}}"#,
    ] {
        run(wrong, &["not an Istanbul coverage-final.json"]);
    }
    run(
        &format!(r#"{{"statementMap":{}}}"#, map(span)),
        &["missing or invalid hit map s"],
    );
    for hits in ["null", "[]"] {
        run(&entry(&map(span), hits), &["missing or invalid hit map s"]);
    }
    for val in [span, "null"] {
        run(
            &entry(&map(val), r#"{"extra":1,"x":0}"#),
            &["statement extra has no span"],
        );
    }
    run(
        &entry(&map(span), "{}"),
        &["invalid hit count", "statement x:"],
    );
    for hits in ["-1", "0.5", "null", "true", r#""1""#, "1e400"] {
        run(
            &entry(&map(span), &map(hits)),
            &["invalid hit count", "statement x:"],
        );
    }
    for bad in ["null", "[]"] {
        for target in [span, r#"{"line":2,"column":3}"#, r#"{"line":2,"column":4}"#] {
            run(
                &entry(&map(&span.replace(target, bad)), &map("0")),
                &["invalid span", "statement x:"],
            );
        }
    }
    for line in ["0", "-0", "1.5", "1e400"] {
        let span = span.replacen(r#""line":2"#, &format!(r#""line":{line}"#), 1);
        run(
            &entry(&map(&span), &map("0")),
            &["invalid start", "statement x:"],
        );
    }
    for col in ["-1", "null", "0.5"] {
        run(
            &entry(
                &map(&span.replace(r#""column":3"#, &format!(r#""column":{col}"#))),
                &map("0"),
            ),
            &["invalid start", "statement x:"],
        );
    }
    for end in [
        r#"{"line":1,"column":null}"#,
        r#"{"line":2.5,"column":4}"#,
        r#"{"line":2,"column":2}"#,
        r#"{"line":3,"column":-1}"#,
        r#"{"line":3,"column":0.5}"#,
        r#"{"line":3,"column":1e400}"#,
        r#"{"line":2}"#,
    ] {
        run(
            &entry(
                &map(&span.replace(r#"{"line":2,"column":4}"#, end)),
                &map("0"),
            ),
            &["invalid end", "statement x:"],
        );
    }
}

#[test]
fn companion_checks_and_sarif_restrictions_precede_comparison_io() {
    let d = fixture();
    for (flags, detail) in [
        (
            vec!["--fail-regression"],
            "--fail-regression needs --baseline",
        ),
        (
            vec!["--show-unchanged"],
            "--show-unchanged needs --baseline",
        ),
        (vec!["--fail-changed"], "--fail-changed needs --base"),
        (
            vec!["--baseline", "b.json", "--fail-changed"],
            "--fail-changed needs --base",
        ),
        (
            vec![
                "--format",
                "sarif",
                "--baseline",
                "b.json",
                "--fail-regression",
            ],
            "SARIF does not support --baseline",
        ),
        (
            vec!["--format", "sarif", "--baseline", "b.json"],
            "SARIF does not support --baseline",
        ),
        (
            vec!["--format", "sarif", "--base", "HEAD"],
            "SARIF does not support --base",
        ),
    ] {
        let mut args = vec!["--coverage", "hits.data"];
        args.extend(flags);
        before_render(d.path(), &args, &[detail]);
    }
}

#[test]
fn an_admitted_baseline_runs_the_comparison() {
    let d = fixture();
    let saved = invoke(
        d.path(),
        &[
            "--coverage",
            "hits.data",
            "--format",
            "json",
            "--output",
            "b.json",
        ],
    );
    check(&saved, 0);
    for gate in [&[][..], &["--fail-regression"]] {
        let args = [
            &["--coverage", "hits.data", "--baseline", "b.json"][..],
            gate,
        ]
        .concat();
        let out = invoke(d.path(), &args);
        check(&out, 0);
        assert_eq!(out.stderr.as_slice(), b"");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.starts_with("No changes since baseline.\n"), "{text}");
    }
    let out = invoke(
        d.path(),
        &[
            "--coverage",
            "hits.data",
            "--baseline",
            "b.json",
            "--format",
            "json",
        ],
    );
    check(&out, 0);
    let doc = common::validated_delta(&out.stdout);
    assert_eq!(doc["summary"]["unchanged"], 3);
}

fn warning_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path();
    source(cwd, ".crapts.toml", "exclude=['old/**']");
    source(
        cwd,
        "recover.js",
        "const x; let [a]; function kept() { if (x) go() }",
    );
    source(cwd, "fatal.ts", "function partial() { if (x) {");
    for path in ["aa/index.ts", "bb/index.ts"] {
        source(cwd, path, "function f() {}");
    }
    source(cwd, "arrows.ts", &"items.map(x => x + 1);\n".repeat(5));
    let mut spans = json!({});
    let mut hits = json!({});
    for line in 1..=5 {
        spans[line.to_string()] =
            json!({"start":{"line":line,"column":0},"end":{"line":line,"column":21}});
        hits[line.to_string()] = json!(1);
    }
    source(
        cwd,
        "hits.data",
        &json!({"arrows.ts":{"statementMap":spans,"s":hits},"index.ts":{"statementMap":{},"s":{}}})
            .to_string(),
    );
    d
}

fn unused_messages(cwd: &Path) -> String {
    format!(
        "warning: {}: exclude 'old/**' excluded no file; globs are relative to the config directory\nwarning: --exclude 'gone/**' excluded no file; globs are relative to the current directory\n",
        cwd.join(".crapts.toml").canonicalize().unwrap().display()
    )
}

#[test]
fn diagnostics_arrive_in_discovery_parse_path_and_producer_order() {
    let d = warning_fixture();
    let out = score(d.path(), &["--exclude", "gone/**", "--format", "json"]);
    check(&out, 0);
    let stderr = String::from_utf8(out.stderr).unwrap();
    let rest = stderr.strip_prefix(&unused_messages(d.path())).unwrap();
    let lines: Vec<_> = rest.lines().collect();
    assert_eq!(lines.len(), 5, "{rest}");
    assert!(rest.ends_with('\n'));
    assert!(
        lines[0].starts_with("warning: fatal.ts: ")
            && lines[1].starts_with("warning: recover.js: parse error:"),
        "{rest}"
    );
    assert_eq!(
        lines[2],
        "warning: coverage key index.ts is ambiguous between aa/index.ts, bb/index.ts and was left unmatched"
    );
    assert_eq!(
        lines[3],
        "warning: only 1 of 4 analyzed files matched the coverage report"
    );
    assert_eq!(
        lines[4],
        "warning: The coverage producer appears to write one statement per line. These callbacks will score 0% while their parents absorb the line."
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["notices"], json!([]));
    assert!(names(&v).contains(&"kept") && !names(&v).contains(&"partial"));
}

#[test]
fn coverage_read_errors_follow_unused_exclusions_and_precede_source_analysis() {
    let d = warning_fixture();
    source(d.path(), "hits.data", "{");
    let errors = before_render(
        d.path(),
        &["--coverage", "hits.data", "--exclude", "gone/**"],
        &["invalid JSON", "hits.data"],
    );
    let rest = errors.strip_prefix(&unused_messages(d.path())).unwrap();
    assert_eq!(rest.lines().count(), 1);
    assert!(!rest.contains("warning:"));
}

#[test]
fn a_complete_report_precedes_the_gate_status() {
    let d = fixture();
    for (threshold, code) in [("6", 1), ("12", 0)] {
        let args = ["--format", "json", "--threshold", threshold, "--fail-above"];
        let expected = score(d.path(), &args);
        check(&expected, code);
        let mut args = args.to_vec();
        args.extend(["--output", "out.json"]);
        let out = score(d.path(), &args);
        check(&out, code);
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
        assert_eq!(
            fs::read(d.path().join("out.json")).unwrap(),
            expected.stdout
        );
    }
}

#[test]
fn a_destination_failure_takes_precedence_at_the_process_boundary() {
    let d = fixture();
    for threshold in ["6", "12"] {
        let args = [
            "--coverage",
            "hits.data",
            "--threshold",
            threshold,
            "--fail-above",
            "--format",
            "json",
        ];
        let out = binary(d.path())
            .args(args)
            .args(["--output", "src"])
            .output()
            .unwrap();
        check(&out, 2);
        let diagnostic = String::from_utf8(out.stderr).unwrap();
        assert!(
            diagnostic.contains("src") && diagnostic.contains("os error"),
            "{diagnostic}"
        );
        #[cfg(unix)]
        {
            use std::os::fd::OwnedFd;
            use std::process::Stdio;
            let (reader, writer) = std::io::pipe().unwrap();
            drop(reader);
            let output = binary(d.path())
                .args(args)
                .stdout(Stdio::from(OwnedFd::from(writer)))
                .output()
                .unwrap();
            check(&output, 2);
            let diagnostic = String::from_utf8(output.stderr).unwrap();
            assert!(
                diagnostic.contains("stdout")
                    && diagnostic.to_ascii_lowercase().contains("broken pipe"),
                "{diagnostic}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn an_injected_writer_uses_pipe_width_and_colour_when_stdout_is_a_terminal() {
    use std::io::IsTerminal;
    const CHILD: &str = "CRAPTS_TEST_INJECTED_SINK";
    if let Some(cwd) = std::env::var_os(CHILD) {
        assert!(std::io::stdout().is_terminal());
        let args = ["--coverage", "hits.data"].map(std::ffi::OsString::from);
        let mut output = Vec::new();
        let mut err = Vec::new();
        let code = crapts::cli::run(Path::new(&cwd), &args, &mut output, &mut err);
        assert_eq!(code, std::process::ExitCode::SUCCESS);
        assert_eq!(err.as_slice(), b"");
        let report = String::from_utf8(output).unwrap();
        assert!(!report.contains("\x1b["), "{report}");
        tier(&report, 0, false);
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    measured_files(dir.path(), &[("f.ts", "f")]);
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "an_injected_writer_uses_pipe_width_and_colour_when_stdout_is_a_terminal",
        "--nocapture",
    ])
    .env(CHILD, dir.path())
    .env("COLUMNS", "59")
    .env_remove("NO_COLOR")
    .env_remove("FORCE_COLOR");
    run_in_pty(cmd);
}

#[test]
fn s12_duplicate_settings_keep_flags_and_provider_specific_default_origins() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("src");
    fs::create_dir(&child).unwrap();
    fs::write(
        dir.path().join(".crapts.toml"),
        "[duplicates]\nenabled=false\nmin-similarity=0.82\n[duplicates.triage]\nprovider='openai'",
    )
    .unwrap();
    let cfg = settings(
        &child,
        &[
            "absent.ts",
            "--coverage",
            "absent.json",
            "--duplicates",
            "--dup-threshold",
            "1",
            "--dup-threshold",
            "0.75",
        ],
    );
    assert_eq!(
        cfg["settings"]["duplicates"],
        json!({
            "enabled":{"value":true,"origin":"flag"},
            "min_similarity":{"value":0.75,"origin":"flag"},
            "min_nodes":{"value":20,"origin":"default"},
            "triage":{
                "enabled":{"value":false,"origin":"default"},
                "provider":{"value":"openai","origin":"config"},
                "model":{"value":"gpt-6-luna","origin":"default"},
                "confidence_floor":{"value":0.5,"origin":"default"},
                "max_requests":{"value":null,"origin":"default"}
            }
        })
    );
}

#[test]
fn s13_invalid_duplicate_config_names_its_source_even_when_overridden() {
    let dir = tempfile::tempdir().unwrap();
    for (group, key, value) in [
        ("duplicates", "min-similarity", "nan"),
        ("duplicates", "min-similarity", "-0.01"),
        ("duplicates", "min-nodes", "-1"),
        ("duplicates", "min-lines", "1"),
        ("duplicates.triage", "confidence-floor", "1.01"),
        ("duplicates.triage", "max-requests", "-1"),
        ("duplicates.triage", "max-requests", "4294967296"),
        ("duplicates.triage", "provider", "'other'"),
    ] {
        fs::write(
            dir.path().join(".crapts.toml"),
            format!("[{group}]\n{key}={value}"),
        )
        .unwrap();
        let out = invoke(dir.path(), &["--print-config", "--dup-threshold", "0.8"]);
        check(&out, 2);
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains(".crapts.toml") && err.contains(key), "{err}");
    }
}

#[test]
fn s39_duplicate_aliases_are_mutually_exclusive_and_accept_closed_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".crapts.toml");
    fs::write(&path, "[duplicates]\nthreshold=0.7\nmin-similarity=0.82").unwrap();
    let rejected = invoke(dir.path(), &["--print-config"]);
    check(&rejected, 2);
    let error = String::from_utf8(rejected.stderr).unwrap();
    assert!(
        error.contains(".crapts.toml") && error.contains("duplicate min-similarity"),
        "{error}"
    );
    for key in ["threshold", "min-similarity", "min_similarity"] {
        fs::write(&path, format!("[duplicates]\n{key}=0.0\nmin_nodes=0\n[duplicates.triage]\nconfidence_floor=1.0\nmax_requests=4294967295")).unwrap();
        let cfg = settings(dir.path(), &[]);
        let duplicates = &cfg["settings"]["duplicates"];
        assert_eq!(
            duplicates["min_similarity"],
            json!({"value":0.0,"origin":"config"})
        );
        assert_eq!(duplicates["min_nodes"]["value"], 0);
        assert_eq!(duplicates["triage"]["confidence_floor"]["value"], 1.0);
        assert_eq!(duplicates["triage"]["max_requests"]["value"], u32::MAX);
    }
}

#[test]
fn s40_a_later_duplicate_threshold_cannot_hide_an_invalid_flag() {
    let dir = tempfile::tempdir().unwrap();
    for value in ["1.1", "-0.1", "NaN", "inf"] {
        let out = invoke(
            dir.path(),
            &[
                "--dup-threshold",
                value,
                "--dup-threshold",
                "0.8",
                "--print-config",
            ],
        );
        check(&out, 2);
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(
            err.contains("--dup-threshold") && err.contains(value),
            "{err}"
        );
    }
}
