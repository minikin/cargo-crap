#[path = "docs/action.rs"]
mod action;
#[path = "docs/check.rs"]
mod check;
#[path = "docs/cli.rs"]
mod cli;
mod common;
#[path = "docs/config.rs"]
mod config;
#[path = "docs/coverage.rs"]
mod coverage;
#[path = "docs/fixture.rs"]
mod fixture;
#[path = "docs/formats.rs"]
mod formats;
#[path = "docs/guides.rs"]
mod guides;
#[path = "docs/index.rs"]
mod index;
#[path = "docs/install.rs"]
mod install;
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
fn s17_omitted_and_explicitly_disabled_signals_match_the_published_cart_reports() {
    let repo = fixture::cart_with(FeatureCoverage::Own).unwrap();
    for explicit in [false, true] {
        if explicit {
            let path = repo.root().join(".crapts.toml");
            let mut cfg = std::fs::read_to_string(&path).unwrap_or_default();
            cfg.push_str("\n[signals]\nenabled = false\n");
            std::fs::write(path, cfg).unwrap();
        }
        for name in ["output-formats", "json"] {
            let page = Page::load(&format!("docs/crapts/reference/{name}.md"));
            let sample = pages::samples(&page)
                .unwrap()
                .into_iter()
                .find(|s| s.marker.effective_format().as_deref() == Some("json"))
                .unwrap();
            let run = repo.crapts(&sample.marker.args());
            check::check_sample(&sample, &run).unwrap();
            let doc: Value = serde_json::from_str(&run.stdout).unwrap();
            assert!(doc["summary"].get("signal_regressions").is_none());
            for row in doc["entries"].as_array().unwrap() {
                for key in ["signals", "baseline_signals", "signal_spans", "reasons"] {
                    assert!(row.get(key).is_none(), "{name}: {key}");
                }
            }
        }
    }
}

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
    let unset = json!({"value":null,"origin":"default"});
    assert_eq!(
        settings["signals"],
        json!({
            "enabled":{"value":false,"origin":"default"},
            "gate":{"value":["nesting","bumps","bool_ops","stubs"],"origin":"default"},
            "limits":{"nesting":unset,"lines":unset,"bumps":unset,"bool_ops":unset}
        })
    );
    assert_eq!(settings.len(), defaults.len() + 1);
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
    assert_eq!(
        repo.git(&["status", "--porcelain", "--untracked-files=all"]),
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
    let checked = check::check_pages(&pages::crapts_pages()).unwrap_or_else(|e| panic!("{e}"));
    assert!(checked.iter().any(|s| s.page == QUICK_START));
    let unmarked = Page::new(QUICK_START, "# crapts\n\nNo samples.\n");
    let message = err(check::check_pages(&[unmarked]));
    assert_names(&message, &["no crapts-output marker"]);
}

#[test]
fn a_drifted_sample_fails_naming_the_page_the_marker_and_the_first_differing_line() {
    let repo = fixture::cart();
    let page = Page::load(QUICK_START);
    let marker = "<!-- crapts-output: --coverage coverage/lcov.info -->";
    let marker_line = page
        .text
        .lines()
        .position(|l| l == marker)
        .expect("the marker")
        + 1;
    let sample = pages::samples(&page)
        .unwrap()
        .into_iter()
        .find(|s| s.line == marker_line)
        .unwrap();
    let (offset, row) = sample
        .block
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("checkout") && l.contains("42.0"))
        .expect("the checkout row of the quick start sample");
    let drifted = page.with(row, &row.replacen("42.0", "41.0", 1));
    let message = err(check::check_page(&drifted, repo));
    assert_names(
        &message,
        &[
            &format!("{QUICK_START}:{marker_line}"),
            marker,
            &format!("line {} of the block differs", offset + 1),
            "41.0",
            "42.0",
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
    let message = err(check::check_page(&wrong, repo));
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
    assert_eq!(check::check_page(&right, repo).unwrap().len(), 1);
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
        "[a-z'\"#&]{1,6}",
        "[a-z]{1,3}&&[a-z]{1,3}",
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
    assert!(check::check_page(&placeholder, repo).is_ok());
    let literal = marked_page(marker, &format!("crapts {VERSION}\n"));
    assert_names(
        &err(check::check_page(&literal, repo)),
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
    let second = Page::new(
        "docs/crapts/drift.md",
        &format!(
            "# Drift\n\n```json\n[{{\"version\": \"<version>\"}}, {{\"version\": \"{VERSION}\"}}]\n```\n"
        ),
    );
    assert_names(
        &err(pages::literal_versions(&second, VERSION)),
        &["docs/crapts/drift.md:4", VERSION],
    );
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
    let text = "src\\cart.ts:1 keeps \\n here\ncrapts 9.8.7\nnot crapts 9.8.7\nother\\src\\cart.ts.bak src\\cart.ts. src\\cart.ts~ file=src\\cart.ts,line=1\nsrc\\cart.ts:10 at the start\nends src\\cart.ts\r\n";
    assert_eq!(
        windows.apply(text, false),
        "src/cart.ts:1 keeps \\n here\ncrapts <version>\nnot crapts 9.8.7\nother\\src\\cart.ts.bak src/cart.ts. src\\cart.ts~ file=src/cart.ts,line=1\nsrc/cart.ts:10 at the start\nends src/cart.ts\r\n"
    );
    assert_eq!(
        unix.apply(text, false),
        text.replace("crapts 9.8.7\nnot", "crapts <version>\nnot")
    );
}

#[test]
fn a_cargo_crap_marker_under_docs_crapts_fails_the_crapts_check() {
    let page = Page::load(guides::BASELINE);
    let line = page.text.lines().count() + 2;
    let added = Page::new(
        &page.path,
        &format!("{}\n<!-- output: --lcov lcov.info -->\n", page.text),
    );
    let message = err(pages::samples(&added));
    assert_names(
        &message,
        &[
            &format!("{}:{line}", guides::BASELINE),
            "would run cargo-crap",
        ],
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
            &format!("{}:{}", guides::BASELINE, line + 1),
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
    assert_eq!(split("a#b & c"), ["a"]);
    assert_eq!(split("a'#'b & c"), ["a#b", "&", "c"]);
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
    assert!(check::check_commands(&pages, repo).unwrap_or_else(|e| panic!("{e}")) > 0);
    let typo = Page::new(
        "docs/crapts/drift.md",
        "```console\n$ crapts --coverage coverage/lcov.info --fail-abvoe\n```\n",
    );
    assert_names(
        &err(check::check_commands(&[typo], repo)),
        &["docs/crapts/drift.md:2", "--fail-abvoe"],
    );
    let help = Page::new(
        "docs/crapts/drift.md",
        "```bash\ncrapts --fail-abvoe --help\n```\n",
    );
    assert_eq!(repo.crapts(&["--fail-abvoe", "--help"]).code, Some(0));
    assert_names(
        &err(check::check_commands(&[help], repo)),
        &["docs/crapts/drift.md:2", "--fail-abvoe"],
    );
    let print = Page::new(
        "docs/crapts/drift.md",
        "```bash\ncrapts --print-config\n```\n",
    );
    assert_eq!(check::check_commands(&[print], repo), Ok(1));
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
    let reference = Page::new(
        QUICK_START,
        "# Q\n\nSee [the guide][g].\n\n[g]: guides/missing.md\n",
    );
    assert_names(
        &err(check::check_links(&[reference])),
        &[&format!("{QUICK_START}:5"), "guides/missing.md"],
    );
    let continued = Page::new(QUICK_START, "# Q\n\n[g]:\n  guides/gone.md \"Title\"\n");
    assert_names(
        &err(check::check_links(&[continued])),
        &[&format!("{QUICK_START}:4"), "guides/gone.md"],
    );
    let angled = Page::new(
        QUICK_START,
        "# Q\n\n[a](<guides/no such.md>) [b]: <guides/also gone.md>\n",
    );
    let targets: Vec<String> = pages::links(&angled).into_iter().map(|(_, t)| t).collect();
    assert_eq!(targets, ["guides/no such.md"]);
    let defined = Page::new(QUICK_START, "# Q\n\n[b]: <guides/also gone.md>\n");
    assert_names(
        &err(check::check_links(&[defined])),
        &["guides/also gone.md"],
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

fn help_text() -> String {
    let run = fixture::Repo::new().crapts(&["--help"]);
    assert_eq!(run.code, Some(0));
    run.stdout
}

fn help_sample(page: &Page) -> pages::Sample {
    pages::samples(page)
        .unwrap()
        .into_iter()
        .find(|s| s.marker == Marker::Help)
        .expect("a crapts-help marker")
}

#[test]
fn the_marked_help_block_equals_crapts_help() {
    let page = Page::load(cli::CLI);
    let repo = fixture::Repo::new();
    let sample = help_sample(&page);
    check::check_sample(&sample, &repo.crapts(&sample.marker.args()))
        .unwrap_or_else(|e| panic!("{e}"));
    let top = sample
        .block
        .lines()
        .find(|l| l.trim_start().starts_with("--top "))
        .expect("the --top line");
    let drifted = page.with(&format!("{top}\n"), "");
    let sample = help_sample(&drifted);
    let message = err(check::check_sample(
        &sample,
        &repo.crapts(&sample.marker.args()),
    ));
    assert_names(&message, &[cli::CLI, "crapts-help", "--top"]);
}

#[test]
fn the_flags_table_lists_every_option_in_help_once() {
    let page = Page::load(cli::CLI);
    let help = help_text();
    cli::check_flag_rows(&page, &help).unwrap_or_else(|e| panic!("{e}"));
    let rows = cli::flags_table(&page).unwrap();
    assert!(
        rows.iter()
            .all(|(long, _, _)| long != "--help" && long != "--version")
    );
    let all_rows = page
        .text
        .lines()
        .find(|l| l.starts_with("| `--all-rows`"))
        .expect("the --all-rows row");
    let removed = page.with(&format!("{all_rows}\n"), "");
    let heading = page.text.lines().position(|l| l == "## Flags").unwrap() + 1;
    assert_names(
        &err(cli::check_flag_rows(&removed, &help)),
        &[&format!("{}:{heading}", cli::CLI), "--all-rows"],
    );
    let doubled = page.with(
        &format!("{all_rows}\n"),
        &format!("{all_rows}\n{all_rows}\n"),
    );
    assert_names(
        &err(cli::check_flag_rows(&doubled, &help)),
        &["--all-rows", "second row"],
    );
}

#[test]
fn the_help_reader_finds_long_flags_and_clap_defaults() {
    let help = "Options:\n      --top <N>\n          Rows\n\n          [default: 5]\n  -q, --quiet\n          Less\n  -h, --help\n          Help\n";
    let options = cli::help_options(help);
    let found: Vec<_> = options
        .iter()
        .map(|o| (o.long.as_str(), o.default.as_deref()))
        .collect();
    assert_eq!(found, [("--top", Some("5")), ("--quiet", None)]);
    let config = serde_json::json!({"settings": {"top": {"value": 9}, "quiet": {"value": true}}});
    let expected = cli::expected_defaults(help, &config);
    assert_eq!(expected["--top"], "5");
    assert_eq!(expected["--quiet"], "on");
}

#[test]
fn the_flags_table_states_the_defaults_the_binary_uses() {
    let page = Page::load(cli::CLI);
    let repo = fixture::Repo::new();
    let help = help_text();
    let printed = repo.crapts(&["--print-config"]);
    assert_eq!(printed.code, Some(0), "{}", printed.stderr);
    let config: serde_json::Value = serde_json::from_str(&printed.stdout).unwrap();
    let expected = cli::expected_defaults(&help, &config);
    for (flag, cell) in [
        ("--threshold", "30"),
        ("--epsilon", "0.01"),
        ("--format", "text"),
        ("--missing", "pessimistic"),
        ("--sort", "crap"),
        ("--top", "none"),
        ("--comment-marker", "<!-- crapts-report -->"),
        ("--all-rows", "off"),
        ("--no-default-excludes", "off"),
        ("--print-config", "off"),
    ] {
        assert_eq!(expected[flag], cell, "{flag}");
    }
    cli::check_defaults(&page, &expected).unwrap_or_else(|e| panic!("{e}"));
    let row = page
        .text
        .lines()
        .find(|l| l.starts_with("| `--threshold"))
        .expect("the --threshold row");
    let drifted = page.with(row, &row.replacen("| 30 |", "| 15 |", 1));
    assert_names(
        &err(cli::check_defaults(&drifted, &expected)),
        &["--threshold", "15", "30"],
    );
    for (value, cell) in [
        (serde_json::json!(null), "none"),
        (serde_json::json!([]), "none"),
        (serde_json::json!(false), "off"),
        (serde_json::json!(true), "on"),
        (serde_json::json!(30.0), "30"),
        (serde_json::json!(0.01), "0.01"),
        (serde_json::json!("crap"), "crap"),
    ] {
        assert_eq!(cli::default_cell(&value), cell);
    }
}

#[test]
fn the_differences_table_names_only_crapts_options() {
    let page = Page::load(cli::CLI);
    let help = help_text();
    assert!(cli::check_differences(&page, &help).unwrap_or_else(|e| panic!("{e}")) > 0);
    let row = page
        .text
        .lines()
        .find(|l| l.contains("`--coverage") && l.contains("`--lcov"))
        .expect("the --coverage row");
    let drifted = page.with(row, &row.replacen("`--coverage", "`--covrage", 1));
    assert_names(
        &err(cli::check_differences(&drifted, &help)),
        &["--covrage"],
    );
    let plain = page.with(row, &row.replacen("LCOV or", "--covrage, LCOV or", 1));
    assert_names(&err(cli::check_differences(&plain, &help)), &["--covrage"]);
    assert_eq!(
        cli::long_flags("`--top 0`, then --help. (--all-rows) --help/--covrage a--b --"),
        ["--top", "--help", "--all-rows", "--help", "--covrage"]
    );
    let slashed = page.with(
        row,
        &row.replacen("LCOV or", "--help/--covrage, LCOV or", 1),
    );
    assert_names(
        &err(cli::check_differences(&slashed, &help)),
        &["--covrage"],
    );
}

fn all_samples() -> Vec<pages::Sample> {
    pages::crapts_pages()
        .iter()
        .flat_map(|p| pages::samples(p).unwrap_or_else(|e| panic!("{e}")))
        .collect()
}

#[test]
fn every_exit_code_in_the_table_is_demonstrated() {
    let page = Page::load(cli::EXIT_CODES);
    let samples = all_samples();
    let codes: Vec<u8> = cli::exit_codes(&page)
        .unwrap()
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(codes, [0, 1, 2]);
    cli::check_exit_codes(&page, &samples).unwrap_or_else(|e| panic!("{e}"));
    let row = page
        .text
        .lines()
        .find(|l| l.starts_with("| `2`"))
        .expect("the row for code 2");
    let added = page.with(row, &format!("{row}\n| `3` | Never. |"));
    assert_names(
        &err(cli::check_exit_codes(&added, &samples)),
        &[cli::EXIT_CODES, "3"],
    );
    let no_stderr: Vec<_> = samples
        .into_iter()
        .filter(|s| s.marker.stream() == Stream::Stdout)
        .collect();
    assert_names(&err(cli::check_exit_codes(&page, &no_stderr)), &["2"]);
}

fn key_row<'a>(
    page: &'a Page,
    key: &str,
) -> &'a str {
    page.text
        .lines()
        .find(|l| l.starts_with(&format!("| `{key}`")))
        .unwrap_or_else(|| panic!("the {key} row"))
}

#[test]
fn the_configuration_example_sets_every_config_key_and_the_binary_accepts_it() {
    let page = Page::load(config::CONFIG);
    let repo = fixture::Repo::new();
    config::check_example(&page, &repo).unwrap_or_else(|e| panic!("{e}"));
    let (_, body, _) = config::example(&page).unwrap();
    let line = body
        .lines()
        .find(|l| l.starts_with("epsilon"))
        .expect("the epsilon line");
    let removed = page.with(&format!("{line}\n"), "");
    assert_names(
        &err(config::check_example(&removed, &repo)),
        &[config::CONFIG, "epsilon"],
    );
    let typo = page.with(line, &line.replacen("epsilon", "epsilom", 1));
    assert_names(&err(config::check_example(&typo, &repo)), &["epsilom"]);
}

#[test]
fn the_per_run_table_names_exactly_the_per_run_settings() {
    let page = Page::load(config::CONFIG);
    config::check_per_run(&page).unwrap_or_else(|e| panic!("{e}"));
    let rows: Vec<String> = config::per_run_rows(&page)
        .unwrap()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert_eq!(rows, config::PER_RUN);
    let epsilon = key_row(&page, "epsilon");
    let base = page
        .text
        .lines()
        .find(|l| l.starts_with("| `base`"))
        .expect("the base row");
    let moved = page
        .with(&format!("{epsilon}\n"), "")
        .with(base, &format!("{base}\n{epsilon}"));
    assert_names(
        &err(config::check_per_run(&moved)),
        &[config::CONFIG, "epsilon"],
    );
}

#[test]
fn the_config_file_refuses_each_per_run_setting_given_a_value_of_its_own_type() {
    let repo = fixture::Repo::new();
    for (key, value) in [
        ("paths", r#"["src"]"#),
        ("coverage", r#""coverage/lcov.info""#),
        ("format", r#""json""#),
        ("summary", "true"),
        ("all-rows", "true"),
        ("output", r#""report.json""#),
        ("repo-url", r#""https://example.com/r""#),
        ("commit-ref", r#""main""#),
        ("comment-marker", r#""<!-- x -->""#),
        ("baseline", r#""crapts-baseline.json""#),
        ("base", r#""main""#),
    ] {
        let run = config::refusal(&repo, &format!("{key} = {value}"));
        assert_eq!(run.code, Some(2), "{key}");
        assert_eq!(run.stdout, "", "{key}");
        let unknown = run.stderr.contains(&format!("unknown key {key}\n"));
        let flag_only = run.stderr.contains(&format!("flag-only key {key}"));
        assert!(unknown || flag_only, "{key}: {}", run.stderr);
        assert!(
            !run.stderr.contains("invalid type"),
            "{key}: {}",
            run.stderr
        );
    }
}

#[test]
fn the_key_table_names_real_aliases_and_the_flag_of_each_key() {
    let page = Page::load(config::CONFIG);
    let repo = fixture::Repo::new();
    let keys: Vec<String> = config::key_rows(&page)
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(keys, config::CONFIG_KEYS);
    assert_eq!(
        config::aliases(&page).unwrap(),
        BTreeSet::from(
            ["default_excludes", "show_unchanged", "uncovered_hints"].map(str::to_owned)
        )
    );
    config::check_keys(&page, &repo).unwrap_or_else(|e| panic!("{e}"));
    let run = config::refusal(&repo, "fail_above = true");
    assert_eq!(run.code, Some(2));
    assert!(run.stderr.contains("fail_above"), "{}", run.stderr);
    let (top, min) = (key_row(&page, "top"), key_row(&page, "min"));
    let swapped = page
        .with(top, &top.replacen("`--top`", "`--min`", 1))
        .with(min, &min.replacen("`--min`", "`--top`", 1));
    assert_names(&err(config::check_keys(&swapped, &repo)), &["top", "min"]);
    let hints = key_row(&page, "uncovered-hints");
    let threshold = key_row(&page, "threshold");
    let two = page.with(
        threshold,
        &threshold.replacen("`--threshold`", "config only", 1),
    );
    assert_names(
        &err(config::check_keys(&two, &repo)),
        &["threshold", "uncovered-hints"],
    );
    assert!(hints.contains("config only"));
}

#[test]
fn a_flag_probe_passes_the_example_value_in_the_flags_shape() {
    let probe = |key, flag, value: toml::Value| config::flag_probe(key, flag, &value);
    assert_eq!(probe("top", "--top", 5.into()), ["--top", "5"]);
    assert_eq!(
        probe("missing", "--missing", "skip".into()),
        ["--missing", "skip"]
    );
    assert_eq!(
        probe("fail-above", "--fail-above", true.into()),
        ["--fail-above"]
    );
    assert_eq!(
        probe(
            "exclude",
            "--exclude",
            toml::Value::Array(vec!["a".into(), "b".into()])
        ),
        ["--exclude", "a", "--exclude", "b"]
    );
    assert_eq!(
        probe(
            "default-excludes",
            "--no-default-excludes",
            toml::Value::Array(vec!["a".into()])
        ),
        ["--no-default-excludes"]
    );
    assert_eq!(
        probe("epsilon", "--epsilon", 0.5.into()),
        ["--epsilon", "0.5", "--baseline", "crapts-baseline.json"]
    );
}

#[test]
fn the_install_page_matches_the_published_package_metadata() {
    let page = Page::load(install::INSTALL);
    let package = install::package_json();
    let rust = install::rust_version();
    install::check_install(&page, &package, &rust).unwrap_or_else(|e| panic!("{e}"));
    let musl = "`@minikin/crapts-linux-x64-musl`";
    let row = page
        .text
        .lines()
        .find(|l| l.contains(musl))
        .expect("the musl row");
    let removed = page.with(&format!("{row}\n"), "");
    assert_names(
        &err(install::check_install(&removed, &package, &rust)),
        &[install::INSTALL, "@minikin/crapts-linux-x64-musl"],
    );
    assert_names(
        &err(install::check_install(&page, &package, "1.0")),
        &[install::INSTALL, "1.0", &rust],
    );
    let mut renamed = package.clone();
    renamed["name"] = "@minikin/other".into();
    assert_names(
        &err(install::check_install(&page, &renamed, &rust)),
        &["@minikin/crapts", "@minikin/other"],
    );
    for command in install::COMMANDS {
        let line = page
            .text
            .lines()
            .find(|l| l.ends_with(command))
            .unwrap_or_else(|| panic!("{command}"));
        let dropped = page.with(&format!("{line}\n"), "");
        assert_names(
            &err(install::check_install(&dropped, &package, &rust)),
            &[command],
        );
    }
}

#[test]
fn the_coverage_guide_pairs_each_producer_with_the_crapts_command_that_reads_its_file() {
    let page = Page::load(coverage::COVERAGE);
    coverage::check_sections(&page).unwrap_or_else(|e| panic!("{e}"));
    for producer in coverage::PRODUCERS {
        assert_eq!(pages::sections(&page, producer).len(), 1, "{producer}");
    }
    let jest = pages::section(&page, "Jest").unwrap();
    let producer = pages::command_lines(&jest)
        .into_iter()
        .map(|(_, l)| l)
        .find(|l| pages::crapts_arguments(l).unwrap().is_none())
        .expect("the Jest producer");
    let removed = page.with(&format!("{producer}\n"), "");
    assert_names(
        &err(coverage::check_sections(&removed)),
        &[coverage::COVERAGE, "Jest", "producer"],
    );
    let vitest = pages::section(&page, "Vitest").unwrap();
    let command = pages::command_lines(&vitest)
        .into_iter()
        .map(|(_, l)| l)
        .find(|l| pages::crapts_arguments(l).unwrap().is_some())
        .expect("the Vitest crapts command");
    let path =
        coverage::coverage_argument(&pages::crapts_arguments(&command).unwrap().unwrap()).unwrap();
    let drifted = page.with(&command, &command.replacen(&path, "out/other.info", 1));
    assert_names(
        &err(coverage::check_sections(&drifted)),
        &["Vitest", &path, "out/other.info"],
    );
}

#[test]
fn the_coverage_guide_reads_both_kinds_of_committed_coverage() {
    let page = Page::load(coverage::COVERAGE);
    for (file, kind) in [
        ("coverage/coverage-final.json", "statements"),
        ("coverage/lcov.info", "lines"),
    ] {
        let (actual, report) = coverage::kind_sample(&page, file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(actual, kind, "{file}");
        assert_eq!(coverage::crap_of(&report, "checkout"), Some(42.0), "{file}");
        assert_eq!(coverage::crap_of(&report, "total"), Some(1.0), "{file}");
    }
}

#[test]
fn the_complexity_reference_tells_optional_chaining_from_nullish_coalescing() {
    let page = Page::load(fixture::COMPLEXITY_PAGE);
    let repo = fixture::complexity();
    let samples = check::check_page(&page, repo).unwrap_or_else(|e| panic!("{e}"));
    let rows = samples
        .iter()
        .map(|s| coverage::text_rows(&s.block))
        .find(|rows| rows.iter().any(|r| r.0 == "firstTag"))
        .expect("a sample with firstTag");
    let row = |name: &str| rows.iter().find(|r| r.0 == name).cloned().unwrap();
    assert_eq!(row("firstTag").1, "1");
    assert_eq!(row("titleOf").1, "2");
    assert!(row("firstTag").2.ends_with("100.0%") && row("titleOf").2.ends_with("100.0%"));
    for heading in [
        "Scored functions",
        "Decision points",
        "Coverage attribution",
    ] {
        pages::section(&page, heading).unwrap_or_else(|e| panic!("{e}"));
    }
    let targets: Vec<String> = pages::links(&page).into_iter().map(|(_, t)| t).collect();
    for spec in [
        "specs/50-crapts-oxc-analysis-and-names.md",
        "specs/52-crapts-coverage-reading-and-attribution.md",
    ] {
        assert!(
            targets.iter().any(|t| t.ends_with(spec)),
            "{spec} in {targets:?}"
        );
    }
}

#[test]
fn a_text_table_reader_returns_function_cc_and_coverage() {
    let block = "\u{2502}   \u{2506} CRAP \u{2506} CC \u{2506} Coverage \u{2506} Function \u{2506} Location \u{2502}\n\u{2502} x \u{2506} 2.0 \u{2506}  2 \u{2506} 100.0% \u{2506} titleOf \u{2506} src/ops.ts:4 \u{2502}\n";
    assert_eq!(
        coverage::text_rows(block),
        [("titleOf".to_owned(), "2".to_owned(), "100.0%".to_owned())]
    );
}

fn guide_samples(page: &Page) -> Vec<pages::Sample> {
    check::check_page(page, fixture::cart()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn the_baseline_guide_shows_a_regression_tripping_the_gate() {
    let page = Page::load(guides::BASELINE);
    let samples = guide_samples(&page);
    let sample = guides::find_sample(&page, &samples, &guides::REGRESSION_ARGS, 1)
        .unwrap_or_else(|e| panic!("{e}"));
    guides::check_regression_sample(sample).unwrap_or_else(|e| panic!("{e}"));
    assert!(guides::check_baseline_writer(&page).unwrap_or_else(|e| panic!("{e}")) > 0);
    let mut drifted = sample.clone();
    drifted.block = sample.block.replacen("+30.0", "+29.0", 1);
    assert_names(
        &err(guides::check_regression_sample(&drifted)),
        &[guides::BASELINE, "checkout"],
    );
    let writer = pages::command_lines(&page)
        .into_iter()
        .map(|(_, l)| l)
        .find(|l| l.contains("--output"))
        .expect("the baseline writer");
    let sliced = page.with(&writer, &writer.replacen(" --all-rows", "", 1));
    assert_names(
        &err(guides::check_baseline_writer(&sliced)),
        &[guides::BASELINE, "--all-rows"],
    );
}

#[test]
fn the_changed_gate_guide_shows_an_edited_function_failing_the_pull_request() {
    let page = Page::load(guides::CHANGED_GATE);
    let samples = guide_samples(&page);
    let changed = guides::find_sample(&page, &samples, &guides::CHANGED_ARGS, 1)
        .unwrap_or_else(|e| panic!("{e}"));
    guides::check_changed_entry(changed).unwrap_or_else(|e| panic!("{e}"));
    let combined = guides::find_sample(&page, &samples, &guides::COMBINED_ARGS, 1)
        .unwrap_or_else(|e| panic!("{e}"));
    guides::check_combined(combined).unwrap_or_else(|e| panic!("{e}"));
    let mut drifted = changed.clone();
    drifted.block = changed
        .block
        .replacen("\"verdict\":\"fail\"", "\"verdict\":\"pass\"", 1);
    assert_names(
        &err(guides::check_changed_entry(&drifted)),
        &["checkout", "pass"],
    );
    assert_names(
        &err(guides::find_sample(
            &page,
            &samples,
            &guides::CHANGED_ARGS,
            0,
        )),
        &[guides::CHANGED_GATE, "expected exit 0"],
    );
}

fn without_sample(
    page: &Page,
    format: &str,
) -> Page {
    let sample = pages::samples(page)
        .unwrap()
        .into_iter()
        .find(|s| s.marker.effective_format().as_deref() == Some(format))
        .unwrap_or_else(|| panic!("a {format} sample"));
    let marker = sample.marker.to_string();
    page.with(&format!("{marker}\n"), "")
        .with(&sample.block, "")
}

#[test]
fn each_output_format_has_a_marked_absolute_sample() {
    let page = Page::load(formats::OUTPUT_FORMATS);
    formats::check_output_formats(&page).unwrap_or_else(|e| panic!("{e}"));
    let removed = without_sample(&page, "sarif");
    assert_names(
        &err(formats::check_output_formats(&removed)),
        &[formats::OUTPUT_FORMATS, "sarif"],
    );
    let compared = Page::new(
        formats::OUTPUT_FORMATS,
        "<!-- crapts-output: --coverage coverage/lcov.info --baseline crapts-baseline.json -->\n```text\n```\n",
    );
    assert_names(
        &err(formats::check_output_formats(&compared)),
        &["--baseline"],
    );
}

#[test]
fn each_marked_json_shaped_sample_carries_the_schema_of_its_format() {
    let page = Page::load(formats::JSON);
    let ids = formats::schema_ids();
    assert_eq!(ids.len(), 3);
    formats::check_cited(&page, &ids).unwrap_or_else(|e| panic!("{e}"));
    let samples = all_samples();
    assert!(formats::check_shapes(&samples, &ids).unwrap_or_else(|e| panic!("{e}")) > 0);
    for format in ["json", "sarif", "shields"] {
        assert!(
            samples
                .iter()
                .any(|s| s.marker.effective_format().as_deref() == Some(format)),
            "{format}"
        );
    }
    let v2 =
        "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/crapts/report-v2.json";
    let cited = Page::new(&page.path, &format!("{}\nSee `{v2}`.\n", page.text));
    assert_names(
        &err(formats::check_cited(&cited, &ids)),
        &[formats::JSON, v2],
    );
    let json = samples
        .iter()
        .find(|s| s.marker.effective_format().as_deref() == Some("json"))
        .unwrap();
    let mut swapped = json.clone();
    let id = ids
        .iter()
        .find(|id| json.block.contains(id.as_str()))
        .unwrap();
    swapped.block = json.block.replacen(id.as_str(), formats::SARIF_SCHEMA, 1);
    assert_names(
        &err(formats::check_shape(&swapped, &ids)),
        &[&json.marker.to_string()],
    );
}

#[test]
fn the_json_reference_tables_the_top_level_fields() {
    let page = Page::load(formats::JSON);
    let (report, delta) = (
        formats::schema("report-v1.json"),
        formats::schema("delta-v1.json"),
    );
    let external = formats::schemas_by_id();
    formats::check_fields(&page, &report, &delta, &external).unwrap_or_else(|e| panic!("{e}"));
    let rows: Vec<String> = formats::field_rows(&page)
        .unwrap()
        .into_iter()
        .map(|r| r.field)
        .collect();
    assert!(rows.contains(&"regression_gate".to_owned()) && rows.contains(&"changed".to_owned()));
    assert!(
        pages::links(&page)
            .iter()
            .any(|(_, t)| t == "../guides/changed-gate.md#combined-with-a-baseline"),
        "the combined sample link"
    );
    let row = page
        .text
        .lines()
        .find(|l| l.starts_with("| `regression_gate`"))
        .expect("the regression_gate row");
    let drifted = page.with(row, &row.replacen("| absent |", "| required |", 1));
    assert_names(
        &err(formats::check_fields(&drifted, &report, &delta, &external)),
        &["regression_gate"],
    );
    let doubled = page.with(row, &format!("{row}\n{row}"));
    assert_names(
        &err(formats::check_fields(&doubled, &report, &delta, &external)),
        &["regression_gate", "second row"],
    );
}

#[test]
fn null_is_found_through_references_and_branches() {
    let root = serde_json::json!({"definitions": {"S": {"oneOf": [{"type": "null"}, {"type": "object"}]}}});
    for (node, expected) in [
        (serde_json::json!({"$ref": "#/definitions/S"}), true),
        (serde_json::json!({"type": ["integer", "null"]}), true),
        (serde_json::json!({"type": "null"}), true),
        (serde_json::json!({"const": null}), true),
        (serde_json::json!({"type": "object"}), false),
        (
            serde_json::json!({"$ref": "https://example.invalid/x.json"}),
            false,
        ),
        (
            serde_json::json!({"$ref": "https://example.invalid/n.json"}),
            true,
        ),
    ] {
        let external = std::collections::BTreeMap::from([(
            "https://example.invalid/n.json".to_owned(),
            serde_json::json!({"type": ["object", "null"]}),
        )]);
        assert_eq!(
            formats::admits_null(&root, &node, &external),
            expected,
            "{node}"
        );
    }
    let real = formats::schemas_by_id();
    let report = formats::schema("report-v1.json");
    assert!(!formats::admits_null(
        &report,
        &report["properties"]["changed"],
        &real
    ));
    let schema = serde_json::json!({"required": ["a"], "properties": {"a": {}, "b": {}}});
    assert_eq!(formats::field_state(&schema, "a"), "required");
    assert_eq!(formats::field_state(&schema, "b"), "optional");
    assert_eq!(formats::field_state(&schema, "c"), "absent");
}

#[test]
fn the_key_table_states_each_keys_type_and_default() {
    let page = Page::load(config::CONFIG);
    let repo = fixture::Repo::new();
    let defaults = config::print_config(&repo, &[]).unwrap();
    config::check_types_and_defaults(&page, &defaults).unwrap_or_else(|e| panic!("{e}"));
    let patterns: Vec<String> = defaults["settings"]["default_excludes"]["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["pattern"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(config::listed_default_excludes(&page).unwrap(), patterns);
    let row = key_row(&page, "threshold");
    let typed = page.with(row, &row.replacen("finite number", "boolean", 1));
    assert_names(
        &err(config::check_types_and_defaults(&typed, &defaults)),
        &["threshold", "boolean"],
    );
    let defaulted = page.with(row, &row.replacen("| 30 |", "| 300 |", 1));
    assert_names(
        &err(config::check_types_and_defaults(&defaulted, &defaults)),
        &["threshold", "300", "30"],
    );
    let short = page.with(row, &row.replacen(" | 30 |", " |", 1));
    assert_names(
        &err(config::key_rows(&short).map(|_| ())),
        &["type and default"],
    );
    for (value, cell) in [
        (serde_json::json!(null), "unset"),
        (serde_json::json!([]), "empty"),
        (serde_json::json!([1, 2]), "the two patterns below"),
        (serde_json::json!(false), "false"),
        (serde_json::json!(0.01), "0.01"),
        (serde_json::json!("crap"), "crap"),
    ] {
        assert_eq!(config::default_cell(&value), cell);
    }
}

fn action_yml() -> action::ActionYml {
    action::read_action(&pages::read("actions/crapts/action.yml"))
}

#[test]
fn the_action_reader_finds_inputs_defaults_and_outputs() {
    let parsed = action::read_action(
        "name: x\ninputs:\n  a:\n    description: A.\n    default: \"1\"\n  b:\n    description: B.\noutputs:\n  c:\n    value: x\nruns:\n  using: composite\n",
    );
    assert_eq!(parsed.inputs["a"].as_deref(), Some("1"));
    assert_eq!(parsed.inputs["b"], None);
    assert_eq!(parsed.outputs, BTreeSet::from(["c".to_owned()]));
    let commented = action::read_action(
        "# lead\ninputs:\n  # a note\n  a:\n    default: x\n\n# between\n  b:\n    default: y\noutputs:\n  # none here\n  c:\n    value: x\n",
    );
    assert_eq!(commented.inputs.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(commented.outputs, BTreeSet::from(["c".to_owned()]));
    let real = action_yml();
    assert!(real.inputs.contains_key("coverage") && real.inputs.contains_key("coverage-command"));
    assert_eq!(action::default_cell(None), "none");
    assert_eq!(action::default_cell(Some("")), "none");
    assert_eq!(action::default_cell(Some(VERSION)), "<version>");
    assert_eq!(action::default_cell(Some("true")), "true");
}

#[test]
fn the_action_guide_matches_actions_crapts_action_yml() {
    let page = Page::load(action::ACTION_GUIDE);
    let declared = action_yml();
    action::check_tables(&page, &declared).unwrap_or_else(|e| panic!("{e}"));
    action::check_with_keys(&page, &declared).unwrap_or_else(|e| panic!("{e}"));
    let row = page
        .text
        .lines()
        .find(|l| l.starts_with("| `fail-changed`"))
        .expect("the fail-changed row");
    let removed = page.with(&format!("{row}\n"), "");
    assert_names(
        &err(action::check_tables(&removed, &declared)),
        &["fail-changed"],
    );
    let doubled = page.with(row, &format!("{row}\n{row}"));
    assert_names(
        &err(action::check_tables(&doubled, &declared)),
        &["fail-changed", "second row"],
    );
    let defaulted = page.with(row, &row.replacen("| `true` |", "| `false` |", 1));
    assert_names(
        &err(action::check_tables(&defaulted, &declared)),
        &["fail-changed", "false", "true"],
    );
    let output = page
        .text
        .lines()
        .find(|l| l.starts_with("| `regressed`"))
        .expect("the regressed row");
    let no_output = page.with(&format!("{output}\n"), "");
    assert_names(
        &err(action::check_tables(&no_output, &declared)),
        &["regressed"],
    );
    let with = page
        .text
        .lines()
        .find(|l| {
            l.trim_start().starts_with("coverage:")
                || l.trim_start().starts_with("coverage-command:")
        })
        .expect("a coverage input");
    let indent = &with[..with.len() - with.trim_start().len()];
    let typo = page.with(with, &format!("{with}\n{indent}covrage: x"));
    assert_names(
        &err(action::check_with_keys(&typo, &declared)),
        &[action::ACTION_GUIDE, "covrage"],
    );
    let depth = page
        .text
        .lines()
        .find(|l| l.trim() == "fetch-depth: 0")
        .expect("fetch-depth on the checkout step");
    assert!(action::check_with_keys(&page.with(depth, depth), &declared).is_ok());
}

#[test]
fn the_action_guide_has_a_complete_pull_request_workflow() {
    let page = Page::load(action::ACTION_GUIDE);
    action::check_complete_workflow(&page).unwrap_or_else(|e| panic!("{e}"));
    let full = page
        .blocks()
        .into_iter()
        .find(|b| {
            let single = Page::new(&page.path, &format!("```yaml\n{}```\n", b.body));
            b.info == "yaml" && action::check_complete_workflow(&single).is_ok()
        })
        .expect("a complete workflow block");
    let guide = Page::new(
        &page.path,
        &format!("# Guide\n\n```yaml\n{}```\n", full.body),
    );
    action::check_complete_workflow(&guide).unwrap();
    let source = guide
        .text
        .lines()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with("coverage:") || t.starts_with("coverage-command:")
        })
        .unwrap();
    let starved = guide.with(&format!("{source}\n"), "");
    assert_names(
        &err(action::check_complete_workflow(&starved)),
        &["coverage", "coverage-command"],
    );
    let write = guide
        .text
        .lines()
        .find(|l| l.trim() == "pull-requests: write")
        .unwrap();
    let silent = guide.with(&format!("{write}\n"), "");
    assert_names(
        &err(action::check_complete_workflow(&silent)),
        &["pull-requests"],
    );
}

#[test]
fn the_workflow_reader_reads_the_subset_and_refuses_the_rest() {
    let nodes = action::parse_yaml(
        "on:\n  pull_request:\n    branches:\n      - main\npermissions:\n  contents: read\njobs:\n  a:\n    steps:\n      - uses: x@1\n        with:\n          k: \"v\"\n      - run: |\n          echo hi: there\n",
        1,
    )
    .unwrap();
    let flow = action::workflow(&nodes, 1).unwrap();
    assert_eq!(flow.triggers, ["pull_request"]);
    assert_eq!(flow.permissions.unwrap()["contents"], "read");
    assert_eq!(flow.jobs.len(), 1);
    assert_eq!(flow.jobs[0].steps.len(), 2);
    assert_eq!(
        flow.jobs[0].steps[0].with,
        [(12, "k".to_owned(), "v".to_owned())]
    );
    let flat = action::parse_yaml("on: [push, pull_request]\n", 1).unwrap();
    assert_eq!(
        action::workflow(&flat, 1).unwrap().triggers,
        ["push", "pull_request"]
    );
    assert_eq!(action::parse_yaml("a: b\n  c: d\n", 1).unwrap_err().0, 2);
    assert_eq!(action::parse_yaml("just text\n", 1).unwrap_err().0, 1);
    assert_eq!(
        action::parse_yaml("on: [pull_request\n", 1).unwrap_err().0,
        1
    );
    for bad_on in ["on: [[pull_request]]\n", "on: pull_request]\n"] {
        let page = Page::new(
            action::ACTION_GUIDE,
            &format!("# G\n\n```yaml\n{bad_on}```\n"),
        );
        assert_names(
            &err(action::workflows(&page).map(|_| ())),
            &[&format!("{}:4", action::ACTION_GUIDE)],
        );
    }
    assert_eq!(action::parse_yaml("a: {b: c}\n", 1).unwrap_err().0, 1);
    let listed = action::parse_yaml(
        "jobs:\n  a:\n    steps:\n      - uses: x@1\n        with:\n          coverage-command: [a, b]\n",
        1,
    )
    .unwrap();
    let (line, message) = action::workflow(&listed, 1).err().unwrap();
    assert_eq!(line, 6);
    assert!(message.contains("coverage-command"), "{message}");
    let page = Page::new(
        action::ACTION_GUIDE,
        "# G\n\n```yaml\non: [pull_request\n```\n",
    );
    assert_names(
        &err(action::workflows(&page).map(|_| ())),
        &[&format!("{}:4", action::ACTION_GUIDE)],
    );
}

const TWO_JOBS: &str = "```yaml
on: pull_request
jobs:
  setup:
    permissions:
      contents: read
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 24
  score:
    permissions:
      contents: read
    steps:
      - uses: minikin/cargo-crap/actions/crapts@crapts-v1
        with:
          coverage: coverage/lcov.info
```
";

#[test]
fn a_complete_workflow_is_judged_per_job() {
    let split = Page::new(action::ACTION_GUIDE, TWO_JOBS);
    assert_names(
        &err(action::check_complete_workflow(&split)),
        &["pull-requests: write"],
    );
    let granted = Page::new(
        action::ACTION_GUIDE,
        &TWO_JOBS.replace(
            "      contents: read\n    steps:\n      - uses: minikin",
            "      contents: read\n      pull-requests: write\n    steps:\n      - uses: minikin",
        ),
    );
    assert_names(
        &err(action::check_complete_workflow(&granted)),
        &["actions/checkout"],
    );
    let inherited = Page::new(
        action::ACTION_GUIDE,
        "```yaml\non: pull_request\npermissions:\n  contents: read\n  pull-requests: write\njobs:\n  a:\n    permissions:\n      contents: read\n    steps:\n      - uses: actions/checkout@v4\n      - uses: actions/setup-node@v4\n        with:\n          node-version: 24\n      - uses: minikin/cargo-crap/actions/crapts@crapts-v1\n        with:\n          coverage: c\n```\n",
    );
    assert_names(
        &err(action::check_complete_workflow(&inherited)),
        &["pull-requests: write"],
    );
    let whole = inherited.with("    permissions:\n      contents: read\n", "");
    action::check_complete_workflow(&whole).unwrap();
}

#[test]
fn the_action_guide_sets_up_the_node_the_action_needs() {
    let page = Page::load(action::ACTION_GUIDE);
    assert!(action::check_node_versions(&page).unwrap_or_else(|e| panic!("{e}")) >= 2);
    let line = page
        .text
        .lines()
        .find(|l| l.trim() == format!("node-version: {}", action::MIN_NODE))
        .expect("a node-version line");
    let old = page.with(line, &line.replace("24", "22"));
    assert_names(
        &err(action::check_node_versions(&old)),
        &[action::ACTION_GUIDE, "22", "24"],
    );
    assert_names(
        &err(action::check_complete_workflow(
            &old.with(line, &line.replace("24", "22")),
        )),
        &["22"],
    );
}

#[test]
fn every_reference_to_the_crapts_action_uses_its_own_path_and_tag() {
    let pages = pages::checked_pages();
    let named: usize = pages
        .iter()
        .map(|p| action::check_uses_lines(p).unwrap_or_else(|e| panic!("{e}")))
        .sum();
    assert!(named > 0);
    let page = Page::load(action::ACTION_GUIDE);
    let wrong = Page::new(
        &page.path,
        &format!(
            "{}\n```yaml\n      - uses: minikin/cargo-crap@v1\n```\n",
            page.text
        ),
    );
    assert_names(
        &err(action::check_uses_lines(&wrong)),
        &[action::ACTION_GUIDE, "minikin/cargo-crap@v1"],
    );
    let quoted = Page::new(
        &page.path,
        "# G\n\n```yaml\n      - uses: \"minikin/cargo-crap@v1\"\n```\n",
    );
    assert_names(
        &err(action::check_uses_lines(&quoted)),
        &[
            &format!("{}:4", action::ACTION_GUIDE),
            "minikin/cargo-crap@v1",
        ],
    );
    let right = Page::new(
        &page.path,
        &format!("```yaml\n- uses: '{}'\n```\n", action::ACTION_USES),
    );
    assert_eq!(action::check_uses_lines(&right), Ok(1));
}

#[test]
fn quoted_arguments_with_shell_operators_survive_a_round_trip() {
    for word in ["a&&b", "x && y", "a|b", "a#b", "it's \"q\""] {
        let marker = Marker::Run {
            stream: Stream::Stdout,
            exit: 0,
            args: vec!["--comment-marker".to_owned(), word.to_owned()],
        };
        assert_eq!(
            markers::parse(&marker.to_string()),
            Ok(marker.clone()),
            "{marker}"
        );
    }
}

#[test]
fn an_absolute_sample_refuses_either_spelling_of_a_comparison() {
    for args in [
        "--base=main",
        "--baseline=crapts-baseline.json",
        "--base main",
    ] {
        let page = Page::new(
            formats::OUTPUT_FORMATS,
            &format!(
                "<!-- crapts-output: --coverage coverage/lcov.info --format shields {args} -->\n```json\n```\n"
            ),
        );
        assert_names(&err(formats::check_output_formats(&page)), &["comparison"]);
    }
}

#[test]
fn every_report_and_delta_sample_validates_against_its_schema() {
    let samples: Vec<_> = all_samples()
        .into_iter()
        .filter(|s| {
            s.marker.stream() == Stream::Stdout
                && s.marker.effective_format().as_deref() == Some("json")
        })
        .collect();
    assert!(samples.len() >= 4);
    for sample in &samples {
        formats::check_against_schema(sample).unwrap_or_else(|e| panic!("{e}"));
    }
    let combined = samples
        .iter()
        .find(|s| s.block.contains("\"changed\": {") && s.block.contains("\"regression_gate\""))
        .expect("the combined sample");
    for (from, to) in [
        (",\"counts\":{", ",\"tallies\":{"),
        (
            "\"name_scheme\":1,\"threshold\"",
            "\"name_scheme\":1,\"extra\":1,\"threshold\"",
        ),
        (
            "\"threshold\":30.0,\"base\"",
            "\"threshold\":\"30\",\"base\"",
        ),
    ] {
        let mut drifted = combined.clone();
        assert!(combined.block.contains(from), "{from}");
        drifted.block = combined.block.replacen(from, to, 1);
        assert_names(
            &err(formats::check_against_schema(&drifted)),
            &[&combined.marker.to_string()],
        );
    }
}

#[test]
fn the_index_links_every_page() {
    let pages = pages::crapts_pages();
    assert!(pages.len() >= 12, "{}", pages.len());
    let index = Page::load(index::INDEX);
    index::check_index(&index, &pages).unwrap_or_else(|e| panic!("{e}"));
    let line = index
        .text
        .lines()
        .find(|l| l.contains("(reference/complexity.md)"))
        .expect("the complexity link");
    let removed = index.with(&format!("{line}\n"), "");
    assert_names(
        &err(index::check_index(&removed, &pages)),
        &[index::INDEX, "docs/crapts/reference/complexity.md"],
    );
}

#[test]
fn the_readme_introduces_crapts_in_one_section() {
    let readme = Page::load("README.md");
    assert_eq!(pages::readme_sections(&readme).len(), 1);
    index::check_readme_section(&readme).unwrap_or_else(|e| panic!("{e}"));
    let section = pages::section(&readme, pages::README_SECTION).unwrap();
    let twice = Page::new("README.md", &format!("{}\n{}", readme.text, section.text));
    assert_names(
        &err(index::check_readme_section(&twice)),
        &["2 sections", pages::README_SECTION],
    );
    let heading = format!("## {}", pages::README_SECTION);
    for (from, to, name) in [
        (
            "(docs/crapts/install.md)",
            "(docs/install.md)",
            "docs/crapts/install.md",
        ),
        (
            "npx @minikin/crapts --coverage",
            "npm install -g @minikin/crapts --coverage",
            "npm install",
        ),
        ("```bash\nnpx @minikin/crapts\n```\n", "", "install line"),
    ] {
        let drifted = readme.with(from, to);
        assert_names(&err(index::check_readme_section(&drifted)), &[name]);
    }
    for marker in [
        "<!-- crapts-output: --top 3 -->",
        "<!-- output: --lcov lcov.info -->",
    ] {
        let marked = readme.with(&heading, &format!("{heading}\n\n{marker}"));
        assert_names(&err(index::check_readme_section(&marked)), &["marker"]);
    }
}

#[test]
fn the_root_docs_check_passes_with_docs_crapts_present() {
    let mut pages = pages::crapts_pages();
    pages.extend(pages::readme_sections(&Page::load("README.md")));
    assert_eq!(index::root_markers(&pages), Vec::<String>::new());
    let mut cargo_crap = vec![Page::load("README.md")];
    for entry in std::fs::read_dir(pages::repo_root().join("docs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "md") {
            let relative = path.strip_prefix(pages::repo_root()).unwrap();
            cargo_crap.push(Page::load(&relative.to_string_lossy().replace('\\', "/")));
        }
    }
    assert!(
        index::root_markers(&cargo_crap).len() >= 2,
        "the README and getting-started samples"
    );
    assert!(!pages::read("tests/docs.rs").contains("crapts"));
}

#[test]
fn a_change_to_the_crapts_docs_or_their_sources_runs_the_crapts_docs_check() {
    let ci = pages::read(index::CI);
    let filters = ["docs", "rust", "workflow"].map(|name| (name, index::filter_globs(&ci, name)));
    assert!(filters.iter().all(|(_, globs)| !globs.is_empty()));
    let jobs = ["docs", "test"].map(|job| {
        let runs = index::job_runs(&ci, job);
        (index::job_condition(&ci, job), runs)
    });
    let targets_run = |path: &str| {
        let set: BTreeSet<&str> = filters
            .iter()
            .filter(|(_, globs)| index::matches(globs, path))
            .map(|(name, _)| *name)
            .collect();
        let mut crapts = false;
        let mut root = false;
        for (condition, runs) in &jobs {
            let on = index::evaluate(condition, &set).unwrap_or_else(|| panic!("{condition}"));
            crapts |= on && runs.iter().any(|r| index::runs_crapts_docs(r));
            root |= on && runs.iter().any(|r| index::runs_root_docs(r));
        }
        (set, crapts, root)
    };
    let (set, crapts, root) = targets_run("docs/crapts/guides/baseline.md");
    assert!(set.contains("docs") && !set.contains("rust"), "{set:?}");
    assert!(crapts && root);
    let (set, crapts, _) = targets_run("actions/crapts/action.yml");
    assert!(set.contains("docs") || set.contains("rust"), "{set:?}");
    assert!(crapts);
    let (_, _, root) = targets_run("README.md");
    assert!(root);
    let docs_job = &jobs[0].1;
    assert!(
        docs_job.iter().any(|r| index::runs_crapts_docs(r)),
        "{docs_job:?}"
    );
    for command in [
        "cargo nextest run --workspace --test docs",
        "cargo nextest run -p cargo-crap -p crapts --test docs",
    ] {
        assert!(
            index::runs_crapts_docs(command) && index::runs_root_docs(command),
            "{command}"
        );
    }
    assert!(!index::runs_crapts_docs("cargo nextest run --test docs"));
    assert!(!index::runs_crapts_docs(
        "echo cargo nextest run --workspace --test docs"
    ));
    assert!(!index::runs_crapts_docs(
        "cargo nextest list --workspace --test docs"
    ));
    assert!(!index::runs_root_docs(
        "cargo nextest list --workspace --test docs"
    ));
    assert!(!index::runs_root_docs(
        "cargo nextest run -p crapts --test docs"
    ));
    let none = BTreeSet::new();
    let docs_only = BTreeSet::from(["docs"]);
    let docs_condition = &jobs[0].0;
    assert_eq!(index::evaluate(docs_condition, &docs_only), Some(true));
    assert_eq!(index::evaluate(docs_condition, &none), Some(false));
    assert_eq!(index::evaluate("false", &none), None);
    assert_eq!(
        index::evaluate("(needs.changes.outputs.docs == 'true'", &docs_only),
        None
    );
}

#[test]
fn the_index_counts_only_links_a_reader_can_follow() {
    let pages = pages::crapts_pages();
    let index = Page::load(index::INDEX);
    let line = index
        .text
        .lines()
        .find(|l| l.contains("(reference/complexity.md)"))
        .expect("the complexity link");
    let unused = index.with(
        line,
        &format!(
            "{}\n\n[unused]: reference/complexity.md",
            line.replace("(reference/complexity.md)", "")
        ),
    );
    assert_names(
        &err(index::check_index(&unused, &pages)),
        &["docs/crapts/reference/complexity.md"],
    );
    let used = index.with(
        line,
        &format!(
            "{}\n\n[what gets a score]: reference/complexity.md",
            line.replace("(reference/complexity.md)", "")
        ),
    );
    index::check_index(&used, &pages).unwrap();
    let shadowed = index.with(
        line,
        "[unused][real]\n\n[unused]: reference/complexity.md\n[real]: reference/cli.md",
    );
    assert_names(
        &err(index::check_index(&shadowed, &pages)),
        &["docs/crapts/reference/complexity.md"],
    );
}

#[test]
fn a_multibyte_angle_link_is_read_whole() {
    let page = Page::new(QUICK_START, "# Q\n\n[x](<\u{e9}.md>) and [y](<b c.md>)\n");
    let targets: Vec<String> = pages::links(&page).into_iter().map(|(_, t)| t).collect();
    assert_eq!(targets, ["\u{e9}.md", "b c.md"]);
}

#[test]
fn a_crlf_page_reads_as_lf() {
    let page = Page::new("docs/crapts/x.md", "# X\r\n\r\n| a |\r\n");
    assert_eq!(page.text, "# X\n\n| a |\n");
}
