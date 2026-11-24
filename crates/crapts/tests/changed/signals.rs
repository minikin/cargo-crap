use super::cli::{Repo, report_of};
use super::common;
use serde_json::{Value, json};
use std::fmt::Write as _;

const OLD: &str = "export function f() {\n  let x = 0\n  x += 1\n  return x\n}\n";
const NEW: &str = "export function f() {\n  let x = 0\n  x += 1\n  debugger; return x\n}\n";

fn covered(repo: &Repo) {
    repo.uncovered();
    repo.write("cov.info", repo.read("cov.info").replace(",0\n", ",1\n"));
}

fn compare(
    repo: &Repo,
    args: &[&str],
    expected: i32,
) -> Value {
    covered(repo);
    let mut command = vec!["--coverage", "cov.info", "--format", "json"];
    command.extend_from_slice(args);
    let out = repo.run_in(&repo.root(), &command);
    common::check(&out, expected);
    report_of(&out.stdout)
}

fn enabled(
    repo: &Repo,
    on: bool,
) {
    repo.write(".crapts.toml", format!("[signals]\nenabled = {on}\n"));
}

fn saved(repo: &Repo) -> Value {
    enabled(repo, true);
    let doc = compare(repo, &[], 0);
    repo.write("baseline.json", doc.to_string());
    doc
}

#[test]
fn s29_git_touch_and_saved_regression_have_independent_verdicts() {
    // Given
    let repo = Repo::with(&[("src/f.ts", OLD)]);
    let baseline = saved(&repo);
    assert_eq!(baseline["entries"][0]["signals"]["stubs"], 0);
    repo.write("src/f.ts", NEW);
    let mut with_git = None;
    let mut saved_entries = None;
    for (flags, reason, changed, code) in [
        (&["--base", "main", "--fail-changed"][..], false, true, 0),
        (
            &["--baseline", "baseline.json", "--fail-regression"],
            true,
            false,
            1,
        ),
        (
            &[
                "--base",
                "main",
                "--baseline",
                "baseline.json",
                "--fail-changed",
            ],
            true,
            true,
            0,
        ),
        (
            &[
                "--base",
                "main",
                "--baseline",
                "baseline.json",
                "--fail-regression",
                "--fail-changed",
            ],
            true,
            true,
            1,
        ),
    ] {
        // When
        let doc = compare(&repo, flags, code);
        // Then
        let entry = &doc["entries"][0];
        assert_eq!(entry["signals"]["stubs"], 1);
        assert_eq!(entry["cyclomatic"], baseline["entries"][0]["cyclomatic"]);
        assert_eq!(entry["crap"], baseline["entries"][0]["crap"]);
        assert_eq!(
            doc["summary"].get("signal_regressions"),
            Some(&if reason { json!(1) } else { Value::Null })
        );
        if reason {
            assert_eq!(
                saved_entries.get_or_insert_with(|| doc["entries"].clone()),
                &doc["entries"]
            );
            assert_eq!(entry["reasons"].as_array().unwrap().len(), 1);
            assert_eq!(entry["reasons"][0]["signal"], "stubs");
            assert_eq!(entry["reasons"][0]["gating"], true);
        } else {
            assert!(entry.get("reasons").is_none());
        }
        assert_eq!(
            doc["summary"]["changed_failures"],
            if changed { json!(0) } else { Value::Null }
        );
        if changed {
            common::compile("schemas/crapts/changed-v1.json")
                .validate(&doc["changed"])
                .unwrap();
            if let Some(previous) = &with_git {
                assert_eq!(previous, &doc["changed"]);
            } else {
                with_git = Some(doc["changed"].clone());
            }
        }
    }
    enabled(&repo, false);
    let off = compare(&repo, &["--base", "main", "--fail-changed"], 0);
    assert_eq!(with_git, Some(off["changed"].clone()));
}

#[test]
fn s30_cross_minor_suppression_leaves_changed_and_absolute_failures_active() {
    // Given
    let repo = Repo::with(&[("src/f.ts", OLD)]);
    let mut baseline = saved(&repo);
    let minor: u32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    baseline["version"] = json!(format!(
        "{}.{}.0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        minor + 1
    ));
    repo.write("baseline.json", baseline.to_string());
    repo.write(
        "src/f.ts",
        format!("{NEW}function fresh() {{if (a) return 1; if (b) return 2; return 0;}}\n"),
    );
    for extra in [None, Some("--fail-changed"), Some("--fail-above")] {
        let mut flags = vec![
            "--base",
            "main",
            "--baseline",
            "baseline.json",
            "--fail-regression",
            "--threshold",
            "2",
        ];
        flags.extend(extra);
        // When
        let doc = compare(&repo, &flags, i32::from(extra.is_some()));
        // Then
        assert_eq!(doc["regression_gate"]["skipped"], true);
        assert_eq!(doc["summary"]["signal_regressions"], 1);
        assert_eq!(doc["summary"]["changed_failures"], 1);
        assert_eq!(doc["summary"]["crappy"], 1);
        let entries = doc["entries"].as_array().unwrap();
        let new = entries
            .iter()
            .find(|entry| entry["function"] == "fresh")
            .unwrap();
        assert_eq!(new["status"], "new");
        assert!(new.get("reasons").is_none());
    }
}

#[test]
fn s58_both_strict_parse_errors_preserve_the_previous_output_file() {
    // Given
    let valid = Repo::with(&[("src/f.ts", OLD)]);
    let base = saved(&valid);
    let broken = "function f() { if (a) {\n";
    for (side, before, after) in [("head", OLD, broken), ("base", broken, NEW)] {
        let repo = Repo::with(&[("src/f.ts", before)]);
        enabled(&repo, true);
        repo.write("baseline.json", base.to_string());
        repo.write("src/f.ts", after);
        repo.write("out.json", "keep");
        // When
        let run = repo.run(&["--baseline", "baseline.json", "--output", "out.json"]);
        // Then
        common::check(&run, 2);
        assert_eq!(run.stdout, b"");
        let err = String::from_utf8(run.stderr).unwrap();
        assert!(err.contains(&format!("{side} src/f.ts")), "{err}");
        assert_eq!(repo.read("out.json"), "keep");
    }
}

fn in_process(
    repo: &Repo,
    flags: &[&str],
    expected: u8,
) -> String {
    let mut args = vec![
        "--coverage",
        "cov.info",
        "--baseline",
        "baseline.json",
        "--fail-regression",
        "--format",
        "pr-comment",
    ];
    args.extend_from_slice(flags);
    let args = args
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = crapts::cli::run(&repo.root(), &args, &mut out, &mut err);
    assert_eq!(
        code,
        std::process::ExitCode::from(expected),
        "{}",
        String::from_utf8_lossy(&err)
    );
    if expected == 2 {
        assert!(String::from_utf8_lossy(&err).contains("65536 UTF-16 code units"));
    }
    String::from_utf8(out).unwrap()
}

fn comment_fixture(n: usize) -> Repo {
    let mut old = String::new();
    let mut baseline = String::new();
    let mut new = String::new();
    for i in 0..n {
        writeln!(old, "function f{i:02}\u{10400}() {{ return 0 }}").unwrap();
        writeln!(
            baseline,
            "function f{i:02}\u{10400}() {{ if (x) return 1; return 0 }}"
        )
        .unwrap();
        writeln!(
            new,
            "function f{i:02}\u{10400}() {{ debugger; if (x) return 1; return 0 }}"
        )
        .unwrap();
    }
    let name = "packages/cart/src/app.ts";
    let repo = Repo::with(&[(name, &old)]);
    repo.write(name, baseline);
    repo.write(".crapts.toml", "threshold = 1\n[signals]\nenabled = true\n");
    let snapshot = compare(&repo, &[], 0);
    repo.write("baseline.json", snapshot.to_string());
    repo.write(name, new);
    covered(&repo);
    repo
}

#[test]
fn s32_the_searchable_comment_folds_reasons_after_25_and_keeps_repository_links() {
    // Given
    let repo = comment_fixture(27);
    // When
    let body = in_process(
        &repo,
        &[
            "--base",
            "main",
            "--comment-marker",
            "<!-- review -->",
            "--repo-url",
            "https://example.invalid/r",
            "--commit-ref",
            "review",
        ],
        1,
    );
    // Then
    assert_eq!(body.lines().next(), Some("<!-- review -->"));
    assert_eq!(body.matches("<!-- review -->").count(), 1);
    let (lead, reasons) = body.split_once("### Harder to maintain").unwrap();
    assert!(lead.find("## Changed functions").unwrap() < lead.find("No CRAP regressions").unwrap());
    let (shown, folded) = reasons
        .split_once("<details><summary>2 more</summary>")
        .unwrap();
    assert_eq!(shown.matches("remove-stub").count(), 25);
    assert_eq!(folded.matches("remove-stub").count(), 2);
    for line in reasons.lines().filter(|s| s.contains("remove-stub")) {
        assert!(line.contains("https://example.invalid/r/blob/review/packages/cart/src/app.ts#L"));
        assert!(line.contains("\x60app.ts:") && !line.contains("\x60packages/cart/"));
    }
    assert!(!body.contains("use --format json --all-rows"));
}

const MARK: &str = "<!-- custom -->";
const BUDGET: usize = 65_536;

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

fn comment(
    repo: &Repo,
    base: bool,
    summary: bool,
    marker: &str,
    reference: &str,
) -> String {
    let mut flags = vec![
        "--comment-marker",
        marker,
        "--repo-url",
        "https://example.invalid/project",
        "--commit-ref",
        reference,
    ];
    if base {
        flags.extend(["--base", "main"]);
    }
    if summary {
        flags.push("--summary");
    }
    in_process(repo, &flags, 1)
}

fn body_parts(
    repo: &Repo,
    reference: &str,
) -> (String, String, String) {
    let full = comment(repo, true, false, MARK, reference);
    let start = full.find("## \u{26a0}").unwrap();
    let changed = full[..start].to_owned();
    let summary = comment(repo, false, true, MARK, reference);
    let core = summary
        .strip_prefix(&format!("{MARK}\n\n"))
        .unwrap()
        .to_owned();
    (full, changed, core)
}

fn reduced(
    changed: &str,
    keep: usize,
    total: usize,
    core: &str,
) -> String {
    if keep == total {
        return format!("{changed}{core}");
    }
    let (table, skipped) = changed.trim_end().split_once("\n\nSkipped: ").unwrap();
    let (lead, table) = table.split_once("\n\n| Verdict |").unwrap();
    let mut out = lead.to_owned();
    if keep > 0 {
        let mut lines = table.lines();
        let headers = lines.by_ref().take(2).collect::<Vec<_>>().join("\n");
        writeln!(out, "\n\n| Verdict |{headers}").unwrap();
        out.push_str(&lines.take(keep).collect::<Vec<_>>().join("\n"));
    }
    let omitted = total - keep;
    let noun = if omitted == 1 {
        "function"
    } else {
        "functions"
    };
    format!("{out}\n\nand {omitted} more changed {noun}\n\nSkipped: {skipped}\n\n{core}")
}

#[test]
fn s33_the_complete_utf16_boundary_and_one_more_unit_choose_different_candidates() {
    // Given
    let repo = comment_fixture(5);
    // When
    let (full, changed, core) = body_parts(&repo, "rev");
    // Then
    assert!(full.contains('\u{10400}') && full.ends_with('\n'));
    let padded = MARK.to_owned() + &"m".repeat(BUDGET - units(&full));
    let exact = full.replacen(MARK, &padded, 1);
    assert_eq!(units(&exact), BUDGET);
    assert_eq!(comment(&repo, true, false, &padded, "rev"), exact);
    let over = padded + "x";
    let expected = format!("{changed}{core}").replacen(MARK, &over, 1);
    assert!(units(&expected) <= BUDGET);
    assert_eq!(comment(&repo, true, false, &over, "rev"), expected);
}

#[test]
fn s34_an_oversized_minimum_preserves_the_report_with_every_summary_and_base_combination() {
    // Given
    let repo = comment_fixture(1);
    let marker = "m".repeat(BUDGET);
    for base in [false, true] {
        for summary in [false, true] {
            let mut flags = vec!["--comment-marker", marker.as_str(), "--output", "out.md"];
            if base {
                flags.extend(["--base", "main"]);
            }
            if summary {
                flags.push("--summary");
            }
            repo.write("out.md", "keep");
            // When
            assert_eq!(in_process(&repo, &flags, 2), "");
            // Then
            assert_eq!(repo.read("out.md"), "keep");
        }
    }
}

#[test]
fn s53_fitting_retains_the_full_counts_at_every_changed_row_cutoff() {
    // Given
    let repo = comment_fixture(5);
    let (full, changed, core) = body_parts(&repo, "rev");
    assert!(full.contains("5 signal regression(s)"));
    for kept in [5, 2, 0] {
        let candidate = reduced(&changed, kept, 5, &core);
        let next = if kept == 5 {
            full.clone()
        } else {
            reduced(&changed, kept + 1, 5, &core)
        };
        let marker = MARK.to_owned() + &"m".repeat(BUDGET - units(&candidate));
        // When
        let expected = candidate.replacen(MARK, &marker, 1);
        // Then
        assert_eq!(units(&expected), BUDGET);
        assert!(units(&next.replacen(MARK, &marker, 1)) > BUDGET);
        assert_eq!(comment(&repo, true, false, &marker, "rev"), expected);
        assert_eq!(expected.matches(&marker).count(), 1);
        assert!(
            expected.contains("5 signal regression(s)")
                && expected.contains("5 fail, 0 over, 0 pass among 5 touched functions")
        );
    }
    let reference = "r".repeat(15_000);
    let expected = comment(&repo, false, true, MARK, &reference);
    assert_eq!(comment(&repo, false, false, MARK, &reference), expected);
    assert!(expected.contains("5 signal regression(s)") && !expected.contains("remove-stub"));
}

#[test]
fn s35_annotations_use_repo_relative_paths_and_only_the_first_gated_site() {
    // Given
    for (filename, declaration, name) in [
        ("a.ts", "function f()", "f"),
        ("a,b.ts", "class C { ['%']()", r#"C::"%25""#),
    ] {
        let method = filename.contains(',');
        let close = if method { "} }" } else { "}" };
        let before = format!("{declaration} {{ return 1 {close}\n");
        let file = format!("packages/cart/src/{filename}");
        let repo = Repo::with(&[(&file, &before)]);
        let cwd = repo.root().join("packages/cart");
        repo.write("packages/cart/.crapts.toml", "[signals]\nenabled=true\n");
        let write_hits = |miss: bool| {
            let mut cov = format!("SF:src/{filename}\n");
            for n in 1..=10 {
                writeln!(cov, "DA:{n},{}", u8::from(!miss)).unwrap();
            }
            cov.push_str("end_of_record\n");
            repo.write("packages/cart/cov.info", cov);
        };
        write_hits(false);
        let absolute = repo.run_in(&cwd, &["--coverage", "cov.info", "--format", "json"]);
        common::check(&absolute, 0);
        let actual = common::validated_report(&absolute.stdout);
        let func = actual["entries"][0]["function"].as_str().unwrap();
        assert_eq!(func, name);
        repo.write("packages/cart/baseline.json", absolute.stdout);
        let head =
            format!("{declaration} {{\n\n\n throw Error(\n 'todo'\n )\n\n\n debugger\n{close}\n");
        repo.write(&file, head);
        write_hits(true);
        let args = [
            "--coverage",
            "cov.info",
            "--baseline",
            "baseline.json",
            "--format",
        ];
        // When
        let out = repo.run_in(&cwd, &[&args[..], &["github"]].concat());
        // Then
        common::check(&out, 0);
        let output = String::from_utf8(out.stdout).unwrap();
        let annotations = output.lines().collect::<Vec<_>>();
        assert_eq!(annotations.len(), 2, "{output}");
        assert!(annotations[0].contains("title=CRAP"));
        assert_eq!(
            annotations[1],
            format!(
                "::warning file=packages/cart/src/{},line=4,endLine=6,title=stubs 0 \u{2192} 2::{} got harder to maintain: remove-stub",
                filename.replace(',', "%2C"),
                func.replace('%', "%25")
            )
        );
        let out = repo.run_in(&cwd, &[&args[..], &["json"]].concat());
        let doc = common::validated_delta(&out.stdout);
        let rs = doc["entries"][0]["reasons"].as_array().unwrap();
        assert_eq!(rs.len(), 3);
        assert_eq!(
            rs.last().unwrap()["spans"],
            json!([{"start":4,"end":6},{"start":9,"end":9}])
        );
    }
}
