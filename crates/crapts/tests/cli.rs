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
