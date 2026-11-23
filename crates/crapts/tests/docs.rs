use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::process::Command;

#[test]
fn cli_help_and_the_printed_settings_cover_the_public_contract() {
    let dir = tempfile::tempdir().unwrap();
    let capture = |arg| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_crapts"));
        cmd.current_dir(dir.path()).arg(arg);
        for key in ["GITHUB_REPOSITORY", "GITHUB_SERVER_URL", "GITHUB_SHA"] {
            cmd.env_remove(key);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{arg}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stderr.as_slice(), b"");
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(
        capture("--version"),
        format!("crapts {}\n", env!("CARGO_PKG_VERSION"))
    );
    let help = capture("--help");
    let listed: Vec<_> = help
        .lines()
        .filter_map(|line| {
            let first = line.split_whitespace().next()?;
            if !first.starts_with('-') {
                return None;
            }
            let name = line
                .split_whitespace()
                .find(|token| token.starts_with("--"))?;
            Some(name.trim_start_matches("--"))
        })
        .collect();
    let public = [
        "coverage",
        "threshold",
        "fail-above",
        "baseline",
        "fail-regression",
        "epsilon",
        "show-unchanged",
        "base",
        "fail-changed",
        "missing",
        "allow",
        "exclude",
        "no-default-excludes",
        "min",
        "top",
        "sort",
        "summary",
        "format",
        "output",
        "repo-url",
        "commit-ref",
        "comment-marker",
        "all-rows",
        "print-config",
        "help",
        "version",
    ];
    assert_eq!(listed.len(), public.len(), "{help}");
    assert_eq!(BTreeSet::from_iter(listed), BTreeSet::from(public));
    assert!(
        help.contains("text") && help.contains("human") && help.contains("--coverage <file>"),
        "{help}"
    );
    let config: Value = serde_json::from_str(&capture("--print-config")).unwrap();
    assert!(config["config_path"].is_null());
    let settings = config["settings"].as_object().unwrap();
    let defaults = [
        ("paths", json!(["."])),
        ("coverage", Value::Null),
        ("threshold", json!(30.0)),
        ("fail_above", json!(false)),
        ("baseline", Value::Null),
        ("fail_regression", json!(false)),
        ("epsilon", json!(0.01)),
        ("show_unchanged", json!(false)),
        ("base", Value::Null),
        ("fail_changed", json!(false)),
        ("missing", json!("pessimistic")),
        ("allow", json!([])),
        ("exclude", json!([])),
        (
            "default_excludes",
            Value::Array(
                [
                    "**/node_modules/**",
                    "**/dist/**",
                    "**/build/**",
                    "**/coverage/**",
                    "**/*.test.*",
                    "**/*.spec.*",
                    "**/*.stories.*",
                    "**/*.min.js",
                    "**/__tests__/**",
                    "**/__mocks__/**",
                ]
                .map(|pattern| json!({"pattern":pattern,"origin":"default"}))
                .to_vec(),
            ),
        ),
        ("min", Value::Null),
        ("top", Value::Null),
        ("sort", json!("crap")),
        ("summary", json!(false)),
        ("format", json!("text")),
        ("output", Value::Null),
        ("repo_url", Value::Null),
        ("commit_ref", Value::Null),
        ("comment_marker", json!("<!-- crapts-report -->")),
        ("uncovered_hints", json!(false)),
    ];
    assert_eq!(settings.len(), defaults.len());
    for (key, value) in defaults {
        let actual = &settings[key];
        assert_eq!(actual.as_object().unwrap().len(), 2);
        assert_eq!(actual["origin"], "default", "{key}");
        if value.is_number() {
            assert_eq!(actual["value"].as_f64(), value.as_f64(), "{key}");
        } else {
            assert_eq!(actual["value"], value, "{key}");
        }
    }
}

#[test]
fn fourteen_config_keys_and_the_documented_aliases_reach_print_config() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".crapts.toml");
    let input = "threshold=21\nfail-above=true\nmissing='skip'\nexclude=['cache/**']\ndefault-excludes=[]\nallow=['get?']\nmin=-1\ntop=1\nsort='file'\nepsilon=0\nfail-regression=true\nshow-unchanged=true\nfail-changed=true\nuncovered-hints=true\n";
    let aliases = input
        .replace("default-excludes", "default_excludes")
        .replace("show-unchanged", "show_unchanged")
        .replace("uncovered-hints", "uncovered_hints");
    let effective = |input: &str| {
        std::fs::write(&file, input).unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_crapts"))
            .current_dir(dir.path())
            .arg("--print-config")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<Value>(&o.stdout).unwrap()
    };
    let canonical = effective(input);
    assert_eq!(canonical, effective(&aliases));
    let source = canonical["settings"]
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(key, s)| (s["origin"] == "config").then_some(key.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        source,
        BTreeSet::from([
            "threshold",
            "fail_above",
            "missing",
            "exclude",
            "default_excludes",
            "allow",
            "min",
            "top",
            "sort",
            "epsilon",
            "fail_regression",
            "show_unchanged",
            "fail_changed",
            "uncovered_hints"
        ])
    );
}
