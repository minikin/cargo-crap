//! Without `--workspace`, cargo at the root of a workspace with a root
//! package selects that package alone and skips every member. The table
//! lists, per recipe and job, the cargo commands that must be there, read
//! as text and looking only at commands that run. It does not cover the
//! `mutants-diff` recipe, CI's `mutants` job or the MSRV job, which have
//! their own checks, and other cargo commands in a listed runner are not
//! checked.

use super::config::{
    VERIFY_PREFIX, block, ci_commands, is_cargo, lines, nextest_pair, read, recipe_commands,
};

#[derive(Clone, Copy)]
enum Source {
    Recipe(&'static str),
    Job(&'static str),
}

struct Runner {
    name: &'static str,
    source: Source,
    /// Each expected command: its tokens after `cargo`, in order.
    expected: &'static [&'static [&'static str]],
}

const COVERAGE: &[&str] = &[
    "llvm-cov",
    "--lcov",
    "--output-path",
    "lcov.info",
    "--workspace",
    "--all-features",
];

const RUNNERS: &[Runner] = &[
    Runner {
        name: "Justfile test",
        source: Source::Recipe("test:"),
        expected: &[
            &["nextest", "run", "--workspace", "--all-targets"],
            &[
                "nextest",
                "run",
                "--workspace",
                "--all-targets",
                "--all-features",
            ],
            &["test", "--doc", "--workspace", "--all-features"],
        ],
    },
    Runner {
        name: "Justfile lint",
        source: Source::Recipe("lint:"),
        expected: &[
            &["fmt", "--all", "--", "--check"],
            &["clippy", "--workspace", "--all-targets"],
            &["clippy", "--workspace", "--all-targets", "--all-features"],
        ],
    },
    Runner {
        name: "Justfile check",
        source: Source::Recipe("check:"),
        expected: &[
            &["check", "--workspace", "--all-targets"],
            &["check", "--workspace", "--all-targets", "--all-features"],
        ],
    },
    Runner {
        name: "Justfile cov",
        source: Source::Recipe("cov:"),
        expected: &[&[
            "llvm-cov",
            "nextest",
            "--workspace",
            "--all-targets",
            "--all-features",
        ]],
    },
    Runner {
        name: "Justfile mutants",
        source: Source::Recipe("mutants FILE:"),
        expected: &[&["mutants", "--workspace", "--file"]],
    },
    Runner {
        name: "Justfile mutants-all",
        source: Source::Recipe("mutants-all:"),
        expected: &[&["mutants", "--workspace"]],
    },
    Runner {
        name: "Justfile crap",
        source: Source::Recipe("crap:"),
        expected: &[COVERAGE],
    },
    Runner {
        name: "Justfile crap-baseline",
        source: Source::Recipe("crap-baseline:"),
        expected: &[COVERAGE],
    },
    Runner {
        name: "Justfile crap-delta",
        source: Source::Recipe("crap-delta:"),
        expected: &[COVERAGE],
    },
    Runner {
        name: "Justfile crap-delta-main",
        source: Source::Recipe("crap-delta-main:"),
        expected: &[COVERAGE],
    },
    Runner {
        name: "CI test",
        source: Source::Job("  test:"),
        expected: &[
            &["nextest", "run", "--workspace", "--all-targets", "--locked"],
            &[
                "nextest",
                "run",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
            ],
            &["test", "--doc", "--workspace", "--all-features", "--locked"],
        ],
    },
    Runner {
        name: "CI lints",
        source: Source::Job("  lints:"),
        expected: &[
            &["fmt", "--all", "--", "--check"],
            &["clippy", "--workspace", "--all-targets"],
            &["clippy", "--workspace", "--all-targets", "--all-features"],
        ],
    },
    Runner {
        name: "CI self_score",
        source: Source::Job("  self_score:"),
        expected: &[COVERAGE],
    },
];

const NARROWING: [&str; 3] = ["--exclude", "-p", "--package"];

fn workspace_problems(
    runner: &str,
    commands: &[Vec<&str>],
    expected: &[&[&str]],
) -> Vec<String> {
    let runs: Vec<&[&str]> = commands
        .iter()
        .flat_map(|c| segments(c))
        .filter_map(cargo_args)
        .collect();
    let mut problems = Vec::new();
    for want in expected {
        let found: Vec<&&[&str]> = runs.iter().filter(|args| matches(args, want)).collect();
        if found.is_empty() {
            problems.push(format!("{runner}: missing `cargo {}`", want.join(" ")));
        }
        for args in found {
            for flag in args.iter().filter(|t| narrows(t)) {
                problems.push(format!(
                    "{runner}: `cargo {}` narrows the workspace with {flag}",
                    args.join(" ")
                ));
            }
        }
    }
    problems
}

/// The shell commands on one line: cut at a comment, split at `&&`, `||`,
/// `;` and `|`, so a token that never reaches cargo cannot satisfy a gate.
fn segments<'a, 'b>(line: &'b [&'a str]) -> Vec<&'b [&'a str]> {
    let end = line
        .iter()
        .position(|t| t.starts_with('#'))
        .unwrap_or(line.len());
    line[..end]
        .split(|t| matches!(*t, "&&" | "||" | ";" | "|"))
        .collect()
}

fn cargo_args<'a, 'b>(command: &'b [&'a str]) -> Option<&'b [&'a str]> {
    let mut i = 0;
    if command.first() == Some(&"env") {
        i = 1;
        while command.get(i) == Some(&"-u") {
            i += 2;
        }
    }
    while command
        .get(i)
        .is_some_and(|t| t.contains('=') && !t.starts_with('-'))
    {
        i += 1;
    }
    (command.get(i) == Some(&"cargo")).then(|| &command[i + 1..])
}

fn narrows(token: &str) -> bool {
    NARROWING.contains(&token)
        || token.starts_with("--exclude=")
        || token.starts_with("--package=")
        || (token.starts_with("-p") && !token.starts_with("--"))
}

/// `args` holds `want`'s tokens in order, and passes `--all-features`
/// exactly when `want` does, so a run with every feature cannot stand in
/// for a deleted default run. `cargo fmt` takes no package selection at
/// all, so its arguments must equal the table's.
fn matches(
    args: &[&str],
    want: &[&str],
) -> bool {
    if want.first() == Some(&"fmt") {
        return args == want;
    }
    let all_features = |tokens: &[&str]| tokens.contains(&"--all-features");
    if all_features(args) != all_features(want) {
        return false;
    }
    let mut rest = args.iter();
    want.iter().all(|w| rest.any(|a| a == w))
}

fn commands_of(source: Source) -> Vec<Vec<String>> {
    let owned = |commands: Vec<Vec<&str>>| -> Vec<Vec<String>> {
        commands
            .into_iter()
            .map(|c| c.into_iter().map(str::to_owned).collect())
            .collect()
    };
    match source {
        Source::Recipe(header) => {
            let justfile = read("Justfile");
            owned(recipe_commands(&block(&lines(&justfile), header)))
        },
        Source::Job(header) => {
            let ci = read(".github/workflows/ci.yml");
            owned(ci_commands(&block(&lines(&ci), header)))
        },
    }
}

fn borrowed(commands: &[Vec<String>]) -> Vec<Vec<&str>> {
    commands
        .iter()
        .map(|c| c.iter().map(String::as_str).collect())
        .collect()
}

#[test]
fn every_gate_recipe_and_job_runs_its_expected_cargo_commands_over_the_workspace() {
    let mut problems = Vec::new();
    for runner in RUNNERS {
        let commands = commands_of(runner.source);
        problems.extend(workspace_problems(
            runner.name,
            &borrowed(&commands),
            runner.expected,
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_spec_44_pins_still_hold_on_the_changed_commands() {
    let justfile = read("Justfile");
    let recipe = recipe_commands(&block(&lines(&justfile), "test:"));
    let ci = read(".github/workflows/ci.yml");
    let job = ci_commands(&block(&lines(&ci), "  test:"));

    for (runner, commands) in [("Justfile test recipe", &recipe), ("CI test job", &job)] {
        for run in nextest_pair(commands, runner) {
            assert!(run.contains(&"--workspace"), "{runner}: {run:?}");
        }
    }
    for run in nextest_pair(&recipe, "Justfile test recipe") {
        assert!(run.starts_with(&VERIFY_PREFIX), "{run:?}");
    }
    for command in job
        .iter()
        .filter(|c| is_cargo(c, &["nextest", "run"]) || is_cargo(c, &["test", "--doc"]))
    {
        assert!(command.contains(&"--locked"), "{command:?}");
    }
}

#[test]
fn a_change_to_a_core_proptest_seed_runs_the_rust_jobs() {
    assert!(any_glob_matches(
        &globs(CI, "            rust:"),
        "crates/cargo-crap-core/proptest-regressions/report/human.txt"
    ));
}

#[test]
fn a_change_to_a_core_test_fixture_runs_the_rust_jobs() {
    let rust = globs(CI, "            rust:");
    for fixture in [
        "crates/cargo-crap-core/src/coverage/p_bad_known.info",
        "crates/cargo-crap-core/src/report/cargo_crap_tool.inc",
        "crates/cargo-crap-core/src/report/second_tool.inc",
    ] {
        assert!(any_glob_matches(&rust, fixture), "{fixture}");
    }
}

#[test]
fn a_change_to_any_golden_proptest_seed_runs_the_rust_jobs() {
    let ci = read(".github/workflows/ci.yml");
    let filter = block(&lines(&ci), "            rust:");
    assert!(
        filter
            .iter()
            .any(|l| l.trim() == "- 'tests/golden/*.proptest-regressions'"),
        "the rust path filter misses tests/golden/*.proptest-regressions"
    );
}

struct Step {
    name: String,
    condition: String,
    env: Vec<String>,
    run: Vec<String>,
}

fn steps(job: &[&str]) -> Vec<Step> {
    /// The block a step's line belongs to: its own keys, a `run: |` body or
    /// an `env:` map, both of which are deeper than the keys.
    enum Inside {
        Keys,
        Run,
        Env,
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut keys_indent = 0;
    let mut inside = Inside::Keys;
    for line in job {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        // A `- ` line opens a step, and what follows the dash is its first
        // key, read like any other: `- if: false` must not lose the `if`.
        let (indent, text) = match text.strip_prefix("- ") {
            Some(rest) => {
                keys_indent = indent + 2;
                inside = Inside::Keys;
                steps.push(Step {
                    name: String::new(),
                    condition: String::new(),
                    env: Vec::new(),
                    run: Vec::new(),
                });
                (keys_indent, rest)
            },
            None => (indent, text),
        };
        let Some(step) = steps.last_mut() else {
            continue;
        };
        if indent > keys_indent {
            match inside {
                Inside::Run if !text.starts_with('#') => step.run.push(text.to_owned()),
                Inside::Env => step.env.push(text.to_owned()),
                _ => {},
            }
            continue;
        }
        inside = Inside::Keys;
        if let Some(name) = text.strip_prefix("name: ") {
            name.clone_into(&mut step.name);
        } else if let Some(condition) = text.strip_prefix("if: ") {
            condition.clone_into(&mut step.condition);
        } else if text == "run: |" {
            inside = Inside::Run;
        } else if text == "env:" {
            inside = Inside::Env;
        } else if let Some(command) = text.strip_prefix("run: ") {
            step.run.push(command.to_owned());
        }
    }
    steps
}

fn step<'a>(
    steps: &'a [Step],
    name: &str,
) -> &'a Step {
    steps
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no step named {name:?}"))
}

fn globs(
    file: &str,
    header: &str,
) -> Vec<String> {
    let text = read(file);
    block(&lines(&text), header)
        .iter()
        .filter_map(|l| l.trim().strip_prefix("- '")?.strip_suffix('\''))
        .map(str::to_owned)
        .collect()
}

/// Whether any glob matches `path`, with `*` kept inside one path
/// component as dorny/paths-filter's picomatch and GitHub's `on.paths` keep
/// it. globset's default lets `*` cross `/`, which would make every filter
/// check here pass too easily.
fn any_glob_matches(
    globs: &[String],
    path: &str,
) -> bool {
    globs.iter().any(|g| {
        globset::GlobBuilder::new(g)
            .literal_separator(true)
            .build()
            .unwrap_or_else(|e| panic!("glob {g:?}: {e}"))
            .compile_matcher()
            .is_match(path)
    })
}

const CI: &str = ".github/workflows/ci.yml";

#[test]
fn cis_mutants_job_runs_the_shared_diff_script() {
    let ci = read(CI);
    let job = steps(&block(&lines(&ci), "  mutants:"));

    let pull_request = step(&job, "Run mutation tests (changed lines only, PRs)");
    let push = step(&job, "Run mutation tests (changed lines only, main)");

    assert_eq!(pull_request.env, ["BASE_REF: ${{ github.base_ref }}"]);
    assert_eq!(
        pull_request.run,
        [r#"bash scripts/gates/mutants-diff.sh --base "origin/$BASE_REF""#],
        "{:?}",
        pull_request.run
    );
    assert_eq!(push.env, ["BEFORE_SHA: ${{ github.event.before }}"]);
    assert!(
        push.run
            .iter()
            .any(|line| line.contains(r#"bash scripts/gates/mutants-diff.sh --base "$BEFORE_SHA""#)),
        "{:?}",
        push.run
    );
    assert!(
        push.run
            .iter()
            .any(|line| line.contains("bash scripts/gates/mutants-diff.sh --base HEAD~1")),
        "{:?}",
        push.run
    );
    assert_eq!(
        pull_request.condition,
        "github.event_name == 'pull_request'"
    );
    assert_eq!(push.condition, "github.event_name == 'push'");

    let full = read(".github/workflows/mutants-full.yml");
    let full_job = steps(&block(&lines(&full), "  mutants:"));
    let shard = step(
        &full_job,
        "Run mutation tests (shard ${{ matrix.shard }}/12)",
    );
    assert_eq!(shard.env, ["SHARD: ${{ matrix.shard }}"]);
    assert_eq!(
        shard.run,
        [r#"cargo mutants --workspace --shard "$SHARD/12" --sharding round-robin"#],
        "{:?}",
        shard.run
    );
    assert!(
        full.contains("shard: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]"),
        "mutants-full.yml must enumerate all 12 0-based shards"
    );
}

#[test]
fn a_change_only_under_crates_runs_the_rust_and_mutants_jobs() {
    for filter in ["            rust:", "            src:"] {
        assert!(
            any_glob_matches(&globs(CI, filter), "crates/cargo-crap-core/src/lib.rs"),
            "the {} filter misses crates/cargo-crap-core/src/lib.rs",
            filter.trim()
        );
    }
}

#[test]
fn a_change_only_to_a_members_cargo_toml_runs_the_rust_jobs() {
    let rust = globs(CI, "            rust:");
    // Then both entries are there, the root's literal one included, so it
    // does not depend on how picomatch reads a leading `**/`
    for entry in ["Cargo.toml", "**/Cargo.toml"] {
        assert!(
            rust.iter().any(|g| g == entry),
            "the rust filter lacks {entry:?}"
        );
    }
    // And the rust filter matches the core's, the root's, and an example
    // package's manifest, which tests/workspace.rs reads
    for manifest in [
        "crates/cargo-crap-core/Cargo.toml",
        "Cargo.toml",
        "examples/hero-shop/Cargo.toml",
    ] {
        assert!(
            any_glob_matches(&rust, manifest),
            "the rust filter misses {manifest}"
        );
    }
}

#[test]
fn a_change_only_to_the_cores_manifest_runs_the_release_script_tests() {
    assert!(any_glob_matches(
        &globs(CI, "            release:"),
        "crates/cargo-crap-core/Cargo.toml"
    ));
}

#[test]
fn release_inputs_and_npm_manifest_changes_run_their_ci_checks() {
    let rust = globs(CI, "            rust:");
    assert!(any_glob_matches(&rust, "npm/crapts/package.json"));
    let release = globs(CI, "            release:");
    for path in [
        ".github/workflows/_build.yml",
        ".github/workflows/release.yml",
        ".github/workflows/release-crapts.yml",
        ".github/workflows/action.yml",
        "crates/crapts/Cargo.toml",
        "crates/crapts/CHANGELOG.md",
        "crates/cargo-crap-core/CHANGELOG.md",
        "Cargo.lock",
        "npm/crapts/package.json",
    ] {
        assert!(
            any_glob_matches(&release, path),
            "release filter misses {path}"
        );
    }
    let npm = globs(CI, "            npm:");
    for path in [
        "npm/crapts/lib/resolve.js",
        "scripts/release/stage-npm.mjs",
        "scripts/release/publish-npm.mjs",
        "scripts/release/package-lock.json",
        "crates/crapts/src/main.rs",
        "crates/cargo-crap-core/src/lib.rs",
        "Cargo.toml",
        "Cargo.lock",
        ".github/workflows/ci.yml",
    ] {
        assert!(any_glob_matches(&npm, path), "npm filter misses {path}");
    }
}

#[test]
fn a_change_only_to_the_mutants_gates_own_files_runs_the_mutants_job() {
    let src = globs(CI, "            src:");
    for path in ["scripts/gates/mutants-diff.sh", ".cargo/mutants.toml"] {
        assert!(any_glob_matches(&src, path), "the src filter misses {path}");
    }
}

#[test]
fn a_change_only_under_crates_runs_the_action_workflow() {
    let action = read(".github/workflows/action.yml");
    let paths: Vec<String> = block(&block(&lines(&action), "  pull_request:"), "    paths:")
        .iter()
        .filter_map(|l| l.trim().strip_prefix("- '")?.strip_suffix('\''))
        .map(str::to_owned)
        .collect();
    assert!(
        any_glob_matches(&paths, "crates/cargo-crap-core/src/lib.rs"),
        "{paths:?}"
    );
}

#[test]
fn candidate_action_inputs_schedule_both_events_and_pin_changes_run_workspace_tests() {
    let action = read(".github/workflows/action.yml");
    for event in ["  pull_request:", "  push:"] {
        let paths: Vec<_> = block(&block(&lines(&action), event), "    paths:")
            .iter()
            .filter_map(|line| line.trim().strip_prefix("- '")?.strip_suffix('\''))
            .map(str::to_owned)
            .collect();
        for (glob, input) in [
            ("actions/crapts/**", "actions/crapts/action.yml"),
            ("scripts/action/crapts/**", "scripts/action/crapts/test.sh"),
            ("npm/crapts/**", "npm/crapts/package.json"),
            ("crates/crapts/**", "crates/crapts/Cargo.toml"),
            (
                ".github/workflows/release-crapts.yml",
                ".github/workflows/release-crapts.yml",
            ),
        ] {
            assert!(
                paths.iter().any(|path| path == glob),
                "{event} misses {glob}"
            );
            assert!(any_glob_matches(&paths, input), "{event} misses {input}");
        }
    }
    let rust = globs(CI, "            rust:");
    for input in ["actions/crapts/action.yml", "npm/crapts/package.json"] {
        assert!(any_glob_matches(&rust, input), "rust filter misses {input}");
    }
}

#[test]
fn a_change_only_to_the_action_workflow_runs_the_check_that_reads_it() {
    assert!(
        any_glob_matches(
            &globs(CI, "            rust:"),
            ".github/workflows/action.yml"
        ),
        "the rust filter misses .github/workflows/action.yml"
    );
}

#[test]
fn just_dev_and_ci_both_run_the_parser_guard() {
    let justfile = read("Justfile");
    let justfile = lines(&justfile);
    let dev = justfile
        .iter()
        .find_map(|l| l.strip_prefix("dev:"))
        .expect("a dev recipe");
    let dev: Vec<&str> = dev.split_whitespace().collect();
    for recipe in ["gates-test", "parser-guard"] {
        assert!(dev.contains(&recipe), "dev does not run {recipe}: {dev:?}");
    }
    let recipe = |header: &str| recipe_commands(&block(&justfile, header));
    assert!(
        recipe("gates-test:").contains(&vec!["bash", "scripts/gates/test.sh"]),
        "gates-test does not run scripts/gates/test.sh"
    );
    assert_eq!(
        recipe("parser-guard:"),
        [vec!["bash", "scripts/gates/parser-guard.sh"]]
    );
    let ci = read(CI);
    let job = steps(&block(&lines(&ci), "  gates:"));
    for command in [
        "bash scripts/gates/test.sh",
        "shellcheck -x scripts/gates/*.sh",
        "bash scripts/gates/parser-guard.sh",
    ] {
        let step = job
            .iter()
            .find(|s| s.run.iter().any(|c| c == command))
            .unwrap_or_else(|| panic!("the gates job does not run {command:?}"));
        // And the step runs whenever the job does: an `if:` on it could
        // switch the command off while the check above still found it
        assert_eq!(
            step.condition, "",
            "the step running {command:?} has a condition"
        );
    }
}

#[test]
fn a_change_only_to_a_gate_script_runs_the_gate_script_tests() {
    let ci = read(CI);
    let job = block(&lines(&ci), "  gates:");
    let condition = job
        .iter()
        .find_map(|l| l.strip_prefix("    if: "))
        .expect("the gates job has a condition");
    assert_eq!(condition, "needs.changes.outputs.rust == 'true'");
    assert!(
        any_glob_matches(
            &globs(CI, "            rust:"),
            "scripts/gates/parser-guard.sh"
        ),
        "the rust filter misses scripts/gates/parser-guard.sh"
    );
}

/// The packages a cargo command selects, in every spelling cargo takes:
/// `-p x`, `-px`, `--package x`, `--package=x`, and `p` inside a cluster
/// of short options, `-vp x`.
fn selected_packages<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut selected = Vec::new();
    let mut tokens = args.iter();
    while let Some(token) = tokens.next() {
        if *token == "--package" {
            selected.extend(tokens.next().copied());
        } else if let Some(name) = token.strip_prefix("--package=") {
            selected.push(name);
        } else if let Some(cluster) = token.strip_prefix('-').filter(|c| !c.starts_with('-')) {
            // A short-option cluster such as `-vp x` or `-qpx`: what follows
            // the `p` is the package, or the next token when nothing does.
            if let Some((_, name)) = cluster.split_once('p') {
                if name.is_empty() {
                    selected.extend(tokens.next().copied());
                } else {
                    selected.push(name);
                }
            }
        }
    }
    selected
}

fn msrv_members(msrv: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = std::process::Command::new(&cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    let members = metadata["workspace_members"].as_array().expect("members");
    let mut names: Vec<String> = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|p| members.contains(&p["id"]) && p["rust_version"] == msrv)
        .map(|p| p["name"].as_str().expect("name").to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_msrv_job_tests_the_1_88_packages_by_name() {
    let ci = read(CI);
    let msrv = block(&lines(&ci), "  msrv:");
    assert!(block(&msrv, "    env:").contains(&"      RUSTUP_TOOLCHAIN: '1.88'"));
    let job = steps(&msrv);
    let cargo: Vec<Vec<&str>> = job
        .iter()
        .flat_map(|s| &s.run)
        .map(|c| c.split_whitespace().collect::<Vec<_>>())
        .filter(|c| cargo_args(c).is_some())
        .collect();
    assert_eq!(cargo.len(), 5, "{cargo:?}");
    // The 1.88 packages are the workspace members that declare 1.88, so a
    // new member at that MSRV cannot be left out of the job unnoticed.
    let msrv_members = msrv_members("1.88");
    assert_eq!(msrv_members, ["cargo-crap", "cargo-crap-core"]);
    let mut standalone = Vec::new();
    let mut joint = 0;
    for command in &cargo {
        let args = cargo_args(command).expect("a cargo run");
        let selected = selected_packages(args);
        if selected == ["cargo-crap-core"] {
            assert!(args.contains(&"--all-targets"), "{command:?}");
            standalone.push(args.windows(2).any(|pair| pair == ["--features", "triage"]));
        } else {
            assert_eq!(selected, msrv_members, "{command:?}");
            joint += 1;
        }
        assert!(args.contains(&"--locked"), "{command:?}");
        assert!(!args.contains(&"--workspace"), "{command:?}");
    }
    assert_eq!(joint, 3);
    standalone.sort_unstable();
    assert_eq!(standalone, [false, true]);
}

#[test]
fn keep_the_package_and_its_parser_independently_buildable() {
    // Given rust-toolchain.toml selects stable, installing the MSRV alone can
    // leave Cargo using stable.
    let ci = read(CI);
    let job = block(&lines(&ci), "  crapts-msrv:");
    assert!(job.contains(&"    needs: [changes]"));
    assert!(job.contains(&"    if: needs.changes.outputs.rust == 'true'"));
    assert!(job.contains(&"      - uses: dtolnay/rust-toolchain@1.98"));
    assert!(block(&job, "    env:").contains(&"      RUSTUP_TOOLCHAIN: '1.98'"));
    let steps = steps(&job);
    assert!(steps.iter().any(|step| {
        step.condition.is_empty()
            && step
                .run
                .iter()
                .any(|line| line == "cargo test --locked -p crapts --all-targets")
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const LINT: &[&[&str]] = &[
        &["fmt", "--all", "--", "--check"],
        &["clippy", "--workspace", "--all-targets"],
        &["clippy", "--workspace", "--all-targets", "--all-features"],
    ];

    const TEST: &[&[&str]] = &[
        &["nextest", "run", "--workspace", "--all-targets"],
        &[
            "nextest",
            "run",
            "--workspace",
            "--all-targets",
            "--all-features",
        ],
    ];

    fn problems(
        recipe: &[&str],
        expected: &[&[&str]],
    ) -> Vec<String> {
        workspace_problems("probe", &recipe_commands(recipe), expected)
    }

    #[test]
    fn a_run_that_excludes_the_core_fails_the_workspace_check() {
        let found = problems(
            &[
                "    cargo nextest run --workspace --exclude cargo-crap-core --all-targets",
                "    cargo nextest run --workspace --all-targets --all-features",
            ],
            TEST,
        );
        assert!(
            found.iter().any(|p| p.contains("--exclude")
                && p.contains("nextest run --workspace --exclude cargo-crap-core")),
            "{found:?}"
        );
    }

    #[test]
    fn a_missing_expected_command_fails_the_workspace_check() {
        let found = problems(
            &[
                "    cargo fmt --all -- --check",
                "    cargo clippy --workspace --all-targets -- -D warnings",
            ],
            LINT,
        );
        assert_eq!(
            found,
            ["probe: missing `cargo clippy --workspace --all-targets --all-features`"]
        );
    }

    #[test]
    fn an_all_features_run_does_not_stand_in_for_a_missing_default_run() {
        let found = problems(
            &[
                "    cargo fmt --all -- --check",
                "    cargo clippy --workspace --all-targets --all-features -- -D warnings",
            ],
            LINT,
        );
        assert_eq!(
            found,
            ["probe: missing `cargo clippy --workspace --all-targets`"]
        );
    }

    #[test]
    fn a_commented_out_run_is_not_a_workspace_run() {
        let found = problems(
            &[
                "    INSTA_UPDATE=no cargo nextest run --workspace --all-targets",
                "    # INSTA_UPDATE=no cargo nextest run --workspace --all-targets --all-features",
            ],
            TEST,
        );
        assert_eq!(
            found,
            ["probe: missing `cargo nextest run --workspace --all-targets --all-features`"]
        );
    }

    #[test]
    fn tokens_out_of_the_table_order_do_not_match() {
        let found = problems(
            &["    cargo nextest run --all-targets --workspace"],
            &[TEST[0]],
        );
        assert_eq!(
            found,
            ["probe: missing `cargo nextest run --workspace --all-targets`"]
        );
    }

    #[test]
    fn a_narrowing_flag_on_an_unexpected_command_is_not_a_problem() {
        let found = problems(
            &[
                "    cargo nextest run --workspace --all-targets",
                "    cargo run --release -- --workspace --exclude 'tests/fixtures/**'",
            ],
            &[TEST[0]],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_joined_package_selector_narrows_the_workspace() {
        for run in [
            "    cargo nextest run --workspace --exclude=cargo-crap-core --all-targets",
            "    cargo nextest run --workspace --package=cargo-crap --all-targets",
            "    cargo nextest run --workspace -pcargo-crap --all-targets",
            "    cargo nextest run --workspace --package cargo-crap --all-targets",
            "    cargo nextest run --workspace -p cargo-crap --all-targets",
        ] {
            let found = problems(&[run], &[TEST[0]]);
            assert!(
                found.iter().any(|p| p.contains("narrows the workspace")),
                "{run}: {found:?}"
            );
        }
    }

    #[test]
    fn a_flag_behind_a_comment_does_not_run() {
        let found = problems(
            &["    cargo mutants # --workspace"],
            &[&["mutants", "--workspace"]],
        );
        assert_eq!(found, ["probe: missing `cargo mutants --workspace`"]);
    }

    #[test]
    fn cargo_as_an_argument_is_not_a_cargo_run() {
        let found = problems(
            &["    echo cargo mutants --workspace"],
            &[&["mutants", "--workspace"]],
        );
        assert_eq!(found, ["probe: missing `cargo mutants --workspace`"]);
    }

    #[test]
    fn a_chained_command_is_read_on_its_own() {
        let found = problems(
            &["    cargo nextest run --lib && echo --workspace --all-targets"],
            &[TEST[0]],
        );
        assert_eq!(
            found,
            ["probe: missing `cargo nextest run --workspace --all-targets`"]
        );
    }

    #[test]
    fn a_command_after_a_chain_is_still_read() {
        let found = problems(
            &["    cd crates && cargo nextest run --workspace --all-targets"],
            &[TEST[0]],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn fmt_arguments_match_the_table_exactly() {
        let found = problems(&["    cargo fmt --all --workspace -- --check"], &[LINT[0]]);
        assert_eq!(found, ["probe: missing `cargo fmt --all -- --check`"]);
    }

    #[test]
    fn the_env_prefix_of_the_test_recipe_is_command_position() {
        let found = problems(
            &[
                "    env -u GOLDEN_RECORD -u INSTA_PENDING_DIR INSTA_UPDATE=no cargo nextest run --workspace --all-targets",
            ],
            &[TEST[0]],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_star_in_a_filter_stays_inside_one_path_component() {
        let star = vec!["crates/*/src/*.rs".to_owned()];
        assert!(any_glob_matches(&star, "crates/x/src/lib.rs"));
        assert!(!any_glob_matches(&star, "crates/x/tests/fixtures/src/y.rs"));
        let root = vec!["Cargo.toml".to_owned()];
        assert!(!any_glob_matches(&root, "crates/x/Cargo.toml"));
    }

    #[test]
    fn every_package_selector_spelling_is_read() {
        let args = [
            "nextest",
            "run",
            "-p",
            "a",
            "-pb",
            "--package",
            "c",
            "--package=d",
            "-vp",
            "e",
            "-qpf",
        ];
        assert_eq!(selected_packages(&args), ["a", "b", "c", "d", "e", "f"]);
    }

    #[test]
    fn a_first_key_is_read_like_any_other() {
        let job = [
            "      - if: false",
            "        run: bash scripts/gates/parser-guard.sh",
            "      - run: |",
            "          first",
            "        name: Block first",
        ];
        let found = steps(&job);
        assert_eq!(found[0].condition, "false");
        assert_eq!(found[0].run, ["bash scripts/gates/parser-guard.sh"]);
        assert_eq!(found[1].run, ["first"]);
        assert_eq!(found[1].name, "Block first");
    }

    #[test]
    fn a_steps_condition_is_read() {
        let job = [
            "      - name: Only on PRs",
            "        if: github.event_name == 'pull_request'",
            "        run: echo",
        ];
        assert_eq!(
            steps(&job)[0].condition,
            "github.event_name == 'pull_request'"
        );
    }

    #[test]
    fn a_nested_name_does_not_rename_the_step() {
        let job = [
            "      - name: Upload",
            "        uses: actions/upload-artifact@v4",
            "        with:",
            "          name: mutants-out",
        ];
        assert_eq!(steps(&job)[0].name, "Upload");
    }

    #[test]
    fn a_comment_in_a_run_block_is_not_a_command() {
        let job = [
            "      - name: Block",
            "        run: |",
            "          # why the next line is there",
            "          first one",
        ];
        assert_eq!(steps(&job)[0].run, ["first one"]);
    }

    #[test]
    fn a_steps_env_is_read() {
        let job = [
            "      - name: With env",
            "        env:",
            "          BASE_REF: ${{ github.base_ref }}",
            "        run: echo",
        ];
        assert_eq!(steps(&job)[0].env, ["BASE_REF: ${{ github.base_ref }}"]);
    }

    #[test]
    fn a_run_block_is_read_line_by_line() {
        let job = [
            "      - name: Block",
            "        run: |",
            "          first one",
            "",
            "          second two",
            "      - name: Line",
            "        run: third",
        ];
        let found = steps(&job);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].run, ["first one", "second two"]);
        assert_eq!(step(&found, "Line").run, ["third"]);
    }

    proptest! {
        #[test]
        fn the_check_names_exactly_the_missing_commands(keep in proptest::collection::vec(any::<bool>(), LINT.len())) {
            let lines: Vec<String> = LINT
                .iter()
                .zip(&keep)
                .filter(|(_, kept)| **kept)
                .map(|(tokens, _)| match tokens[0] {
                    "fmt" => format!("    cargo {}", tokens.join(" ")),
                    _ => format!("    cargo {} -- -D warnings", tokens.join(" ")),
                })
                .collect();
            let recipe: Vec<&str> = lines.iter().map(String::as_str).collect();
            let expected_missing: Vec<String> = LINT
                .iter()
                .zip(&keep)
                .filter(|(_, kept)| !**kept)
                .map(|(tokens, _)| format!("probe: missing `cargo {}`", tokens.join(" ")))
                .collect();
            prop_assert_eq!(problems(&recipe, LINT), expected_missing);
        }
    }
}
