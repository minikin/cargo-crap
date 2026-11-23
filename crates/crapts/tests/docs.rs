#[path = "docs/check.rs"]
mod check;
#[path = "docs/fixture.rs"]
mod fixture;
#[path = "docs/markers.rs"]
mod markers;
#[path = "docs/normalize.rs"]
mod normalize;
#[path = "docs/pages.rs"]
mod pages;
mod support;

use fixture::{FeatureCoverage, VERSION};
use markers::{Marker, Stream};
use normalize::Normalizer;
use pages::Page;
use proptest::prelude::*;
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

const QUICK_START: &str = "docs/crapts/README.md";

fn err<T>(result: Result<T, String>) -> String {
    match result {
        Ok(_) => panic!("the check passed"),
        Err(message) => message,
    }
}

fn assert_names(
    message: &str,
    names: &[&str],
) {
    for name in names {
        assert!(message.contains(name), "`{name}` missing from: {message}");
    }
}

#[test]
fn the_fixture_repository_scores_each_revision_with_its_own_coverage() {
    let repo = fixture::cart();
    let root = repo.root();
    assert_eq!(
        std::fs::read(root.join("coverage/lcov.info")).unwrap(),
        std::fs::read(root.join("coverage/lcov.data")).unwrap()
    );
    let status = Command::new("git")
        .current_dir(&root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        "?? coverage/lcov.info\n?? crapts-baseline.json\n"
    );
    let message = err(fixture::cart_with(FeatureCoverage::KeptFromMain));
    assert_names(&message, &["feature", "checkout", "expected CC 6 CRAP 42"]);
}

#[test]
fn the_score_check_names_the_function_and_both_scores() {
    let report = r#"{"entries":[{"function":"checkout","cyclomatic":6,"crap":24.4}]}"#;
    assert!(fixture::expect_scores("x", report, &[("checkout", 6, 24.4)]).is_ok());
    let message = err(fixture::expect_scores(
        "x",
        report,
        &[("checkout", 6, 42.0)],
    ));
    assert_names(&message, &["checkout", "24.4", "42"]);
    let message = err(fixture::expect_scores("x", report, &[("total", 1, 1.0)]));
    assert_names(&message, &["no row for total"]);
}

#[test]
fn every_marked_sample_in_docs_crapts_matches_a_real_run() {
    let repo = fixture::cart();
    let checked =
        check::check_pages(&pages::crapts_pages(), &repo).unwrap_or_else(|e| panic!("{e}"));
    assert!(checked.iter().any(|s| s.page == QUICK_START));
    let unmarked = Page::new(QUICK_START, "# crapts\n\nNo samples.\n");
    let message = err(check::check_pages(&[unmarked], &repo));
    assert_names(&message, &["no crapts-output marker"]);
}

#[test]
fn a_drifted_sample_fails_naming_the_page_the_marker_and_the_first_differing_line() {
    let repo = fixture::cart();
    let page = Page::load(QUICK_START);
    let row = page
        .text
        .lines()
        .find(|l| l.contains("checkout") && l.contains("42.0"))
        .expect("the checkout row of the quick start sample");
    let drifted = page.with(row, &row.replacen("42.0", "41.0", 1));
    let message = err(check::check_page(&drifted, &repo));
    assert_names(
        &message,
        &[
            QUICK_START,
            "--coverage coverage/lcov.info",
            "41.0",
            "42.0",
            "line ",
        ],
    );
}

fn marked_page(
    marker: &str,
    block: &str,
) -> Page {
    Page::new(
        "docs/crapts/drift.md",
        &format!("# Drift\n\n{marker}\n\n```text\n{block}```\n"),
    )
}

#[test]
fn a_markers_exit_code_is_checked_as_well_as_its_output() {
    let repo = fixture::cart();
    let stdout = repo.crapts(&["--coverage", "coverage/lcov.info"]).stdout;
    let wrong = marked_page(
        "<!-- crapts-output exit=1: --coverage coverage/lcov.info -->",
        &stdout,
    );
    let message = err(check::check_page(&wrong, &repo));
    assert_names(
        &message,
        &[
            "--coverage coverage/lcov.info",
            "expected exit 1",
            "actual exit 0",
        ],
    );
    let right = marked_page(
        "<!-- crapts-output exit=1: --coverage coverage/lcov.info --fail-above -->",
        &stdout,
    );
    assert_eq!(check::check_page(&right, &repo).unwrap().len(), 1);
}

#[test]
fn a_marker_must_be_followed_by_its_fenced_block() {
    let page = Page::new(
        "docs/crapts/drift.md",
        "# Drift\n\n<!-- crapts-output: --coverage coverage/lcov.info -->\nSome prose.\n\n```text\nx\n```\n",
    );
    let message = err(pages::samples(&page));
    assert_names(
        &message,
        &["docs/crapts/drift.md:3", "not followed by its fenced block"],
    );
}

#[test]
fn a_malformed_crapts_marker_fails_instead_of_being_skipped() {
    let cases = [
        ("<!-- crapts-outptu: --top 3 -->", "an unknown marker kind"),
        (
            "<!-- crapts-output exit=x: --top 0 -->",
            "an exit code that is not 0, 1 or 2",
        ),
        (
            "<!-- crapts-stderr: --top 0 -->",
            "a stderr marker without exit=",
        ),
        (
            "<!-- crapts-output: --comment-marker 'a -->",
            "an unterminated quote",
        ),
        (
            "<!-- crapts-help exit=1 -->",
            "a help marker with an exit code",
        ),
        (
            "<!-- crapts-help: --top 3 -->",
            "a help marker with arguments",
        ),
    ];
    for (line, reason) in cases {
        let page = Page::new(
            "docs/crapts/drift.md",
            &format!("# Drift\n\n{line}\n\n```text\n```\n"),
        );
        let message = err(pages::samples(&page));
        assert_names(&message, &["docs/crapts/drift.md:3", reason]);
    }
    let fenced = Page::new(
        "docs/crapts/drift.md",
        "# Drift\n\n```markdown\n<!-- crapts-report -->\n```\n",
    );
    assert!(pages::samples(&fenced).unwrap().is_empty());
}

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9./=_-]{1,8}",
        "--[a-z]{1,8}",
        "[a-z]{1,4}--[a-z]{1,4}",
        "[a-z]{1,4}( [a-z<>!-]{1,4}){1,3}",
    ]
}

fn marker() -> impl Strategy<Value = Marker> {
    let args = prop::collection::vec(word(), 1..6);
    let stream = prop_oneof![Just(Stream::Stdout), Just(Stream::Stderr)];
    prop_oneof![
        Just(Marker::Help),
        (stream, 0u8..=2, args).prop_map(|(stream, exit, args)| Marker::Run { stream, exit, args }),
    ]
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn any_legal_marker_survives_a_format_and_parse_round_trip(marker in marker()) {
        let line = marker.to_string();
        prop_assert_eq!(markers::parse(&line), Ok(marker.clone()), "{}", line);
        if let Marker::Run { stream: Stream::Stdout, exit: 0, .. } = marker {
            prop_assert!(!line.contains("exit="), "{}", line);
        }
    }
}

#[test]
fn an_explicit_exit_0_parses_to_the_marker_without_it() {
    assert_eq!(
        markers::parse("<!-- crapts-output exit=0: --top 3 -->"),
        markers::parse("<!-- crapts-output: --top 3 -->")
    );
    assert_eq!(
        markers::parse("<!-- crapts-output: --top 3 -->")
            .unwrap()
            .to_string(),
        "<!-- crapts-output: --top 3 -->"
    );
}

#[test]
fn the_crapts_version_is_written_as_a_placeholder() {
    let repo = fixture::cart();
    let marker = "<!-- crapts-output: --version -->";
    let placeholder = marked_page(marker, "crapts <version>\n");
    assert!(check::check_page(&placeholder, &repo).is_ok());
    let literal = marked_page(marker, &format!("crapts {VERSION}\n"));
    assert_names(
        &err(check::check_page(&literal, &repo)),
        &["--version", VERSION],
    );
    let page = Page::new(
        "docs/crapts/drift.md",
        &format!("# Drift\n\n```json\n{{\n  \"version\": \"{VERSION}\"\n}}\n```\n"),
    );
    assert_names(
        &err(pages::literal_versions(&page, VERSION)),
        &["docs/crapts/drift.md:5", VERSION],
    );
    let placeholder = page.with(&format!("\"{VERSION}\""), "\"<version>\"");
    assert!(pages::literal_versions(&placeholder, VERSION).is_ok());
}

#[test]
fn normalization_touches_only_native_paths_and_version_positions() {
    let windows = Normalizer {
        windows: true,
        version: "9.8.7",
        fixture_paths: &["src/cart.ts"],
    };
    let captured = r#"{"version": "9.8.7", "entries": [{"file": "src\\cart.ts", "reason": "a \"q\"\n 9.8.7 src\\cart.ts"}]}"#;
    let expected = r#"{"version": "<version>", "entries": [{"file": "src/cart.ts", "reason": "a \"q\"\n 9.8.7 src\\cart.ts"}]}"#;
    assert_eq!(windows.apply(captured, true), expected);
    let nested = r#"{"regression_gate": {"version": "9.8.7", "baseline_version": "9.8.7"}, "changed": {"version": "9.8.7", "entries": [{"version": "9.8.7", "previous_file": "a\\b"}]}, "runs": [{"tool": {"driver": {"version": "9.8.7"}}, "results": [{"uri": "src\\cart.ts"}]}]}"#;
    let normalized = windows.apply(nested, true);
    assert_eq!(normalized.matches("<version>").count(), 4, "{normalized}");
    assert!(
        normalized.contains(r#"[{"version": "9.8.7", "previous_file": "a/b"}]"#),
        "{normalized}"
    );
    assert!(
        normalized.contains(r#""uri": "src/cart.ts""#),
        "{normalized}"
    );
    let unix = Normalizer {
        windows: false,
        ..windows
    };
    assert_eq!(
        unix.apply(r#"{"file": "src\\cart.ts"}"#, true),
        r#"{"file": "src\\cart.ts"}"#
    );
    let text = "src\\cart.ts:1 keeps \\n here\ncrapts 9.8.7\nnot crapts 9.8.7\n";
    assert_eq!(
        windows.apply(text, false),
        "src/cart.ts:1 keeps \\n here\ncrapts <version>\nnot crapts 9.8.7\n"
    );
    assert_eq!(
        unix.apply(text, false),
        text.replace("crapts 9.8.7\nnot", "crapts <version>\nnot")
    );
}

#[test]
fn a_cargo_crap_marker_under_docs_crapts_fails_the_crapts_check() {
    let page = Page::load(QUICK_START);
    let line = page.text.lines().count() + 2;
    let added = Page::new(
        &page.path,
        &format!("{}\n<!-- output: --lcov lcov.info -->\n", page.text),
    );
    let message = err(pages::samples(&added));
    assert_names(
        &message,
        &[&format!("{QUICK_START}:{line}"), "would run cargo-crap"],
    );
    let fenced = Page::new(
        &page.path,
        &format!(
            "{}\n```text\n<!-- output: --lcov lcov.info -->\n```\n",
            page.text
        ),
    );
    let message = err(pages::samples(&fenced));
    assert_names(
        &message,
        &[
            &format!("{QUICK_START}:{}", line + 1),
            "would run cargo-crap",
        ],
    );
}

#[test]
fn the_splitter_groups_quotes_and_stops_at_shell_operators() {
    let split = |s| markers::split(s).unwrap();
    assert_eq!(split("a  'b c' \"d e\"f"), ["a", "b c", "d ef"]);
    assert_eq!(split("a | b"), ["a"]);
    assert_eq!(split("a > out"), ["a"]);
    assert_eq!(split("a && b"), ["a"]);
    assert_eq!(split("a; b"), ["a"]);
    assert_eq!(split("a # note"), ["a"]);
    assert_eq!(split("a#b & c"), ["a#b", "&", "c"]);
    assert_eq!(split("'' x"), ["", "x"]);
    assert_eq!(split("'a|b>c;d#e'"), ["a|b>c;d#e"]);
    assert!(markers::split("a 'b").is_err());
}

#[test]
fn documented_command_lines_are_selected_joined_and_stripped() {
    let page = Page::new(
        "docs/crapts/x.md",
        "```bash\n$ crapts --coverage a \\\n    --top 3\nnpx @minikin/crapts@1.2.3 --summary\nnpx @minikin/crapts-linux-x64\ncraptsy --x\nnpm install -D @minikin/crapts\n```\n\n```toml\ncrapts --nope\n```\n",
    );
    let lines = pages::command_lines(&page);
    assert_eq!(lines[0], (2, "crapts --coverage a --top 3".to_owned()));
    let args: Vec<_> = lines
        .iter()
        .filter_map(|(_, l)| pages::crapts_arguments(l).unwrap())
        .collect();
    assert_eq!(
        args,
        [vec!["--coverage", "a", "--top", "3"], vec!["--summary"]]
    );
    assert_eq!(
        pages::crapts_arguments("npx @minikin/crapts").unwrap(),
        Some(vec![])
    );
    let runs = |line: &str| pages::command_runs(&pages::crapts_arguments(line).unwrap().unwrap());
    assert_eq!(runs("crapts --print-config"), [["--print-config"]]);
    assert_eq!(
        runs("crapts --top 3 --help"),
        [
            vec!["--print-config", "--top", "3"],
            vec!["--top", "3", "--help"]
        ]
    );
}

#[test]
fn every_documented_crapts_command_line_is_accepted_by_the_binary() {
    let repo = fixture::cart();
    let pages = pages::checked_pages();
    assert!(check::check_commands(&pages, &repo).unwrap_or_else(|e| panic!("{e}")) > 0);
    let typo = Page::new(
        "docs/crapts/drift.md",
        "```console\n$ crapts --coverage coverage/lcov.info --fail-abvoe\n```\n",
    );
    assert_names(
        &err(check::check_commands(&[typo], &repo)),
        &["docs/crapts/drift.md:2", "--fail-abvoe"],
    );
    let help = Page::new(
        "docs/crapts/drift.md",
        "```bash\ncrapts --fail-abvoe --help\n```\n",
    );
    assert_eq!(repo.crapts(&["--fail-abvoe", "--help"]).code, Some(0));
    assert_names(
        &err(check::check_commands(&[help], &repo)),
        &["docs/crapts/drift.md:2", "--fail-abvoe"],
    );
    let print = Page::new(
        "docs/crapts/drift.md",
        "```bash\ncrapts --print-config\n```\n",
    );
    assert_eq!(check::check_commands(&[print], &repo), Ok(1));
}

#[test]
fn every_relative_link_resolves() {
    let pages = pages::checked_pages();
    assert!(check::check_links(&pages).unwrap_or_else(|e| panic!("{e}")) > 0);
    let broken = Page::load(QUICK_START);
    let broken = Page::new(
        &broken.path,
        &format!("{}\nSee [the guide](guides/basline.md).\n", broken.text),
    );
    assert_names(
        &err(check::check_links(&[broken])),
        &[QUICK_START, "guides/basline.md"],
    );
    let anchor = Page::new(
        QUICK_START,
        "# Quick start\n\n[a](#quick-start) [b](#nowhere) `[c](#code)`\n",
    );
    assert_names(&err(check::check_links(&[anchor])), &["#nowhere"]);
}

#[test]
fn headings_give_github_anchors() {
    assert_eq!(
        pages::slug("Differences from cargo-crap"),
        "differences-from-cargo-crap"
    );
    assert_eq!(
        pages::slug("`--base` and `--fail-changed`"),
        "--base-and---fail-changed"
    );
    let anchors = pages::anchors("# A\n\n```md\n# Not\n```\n\n## A\n");
    assert_eq!(anchors, BTreeSet::from(["a".to_owned(), "a-1".to_owned()]));
}
