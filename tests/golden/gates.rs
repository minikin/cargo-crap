//! The cargo gates in the table below run over the whole workspace.
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

/// Where a runner's commands come from.
#[derive(Clone, Copy)]
enum Source {
    /// A Justfile recipe, by its header line.
    Recipe(&'static str),
    /// A job in `.github/workflows/ci.yml`, by its header line.
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

/// A package selection that would narrow `--workspace` back down.
const NARROWING: [&str; 3] = ["--exclude", "-p", "--package"];

/// What `runner`'s cargo commands lack or narrow: one line per problem,
/// empty when every expected command is there over the whole workspace.
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

/// The arguments of a command that runs cargo: `cargo` in command position,
/// after at most an `env` with `-u NAME` options and `NAME=value` settings.
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

/// A package selection that would narrow `--workspace` back down, in its
/// separate or joined spelling: `--exclude x`, `--exclude=x`, `-px`.
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
    // Given the expected commands, per Justfile recipe and CI job
    let mut problems = Vec::new();
    for runner in RUNNERS {
        // When the workspace check reads the recipe or job
        let commands = commands_of(runner.source);
        problems.extend(workspace_problems(
            runner.name,
            &borrowed(&commands),
            runner.expected,
        ));
    }
    // Then each expected command is present, its tokens in the table's order,
    // and none of them narrows the package selection
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_spec_44_pins_still_hold_on_the_changed_commands() {
    // Given the Justfile test recipe and the CI test job after this change
    let justfile = read("Justfile");
    let recipe = recipe_commands(&block(&lines(&justfile), "test:"));
    let ci = read(".github/workflows/ci.yml");
    let job = ci_commands(&block(&lines(&ci), "  test:"));

    // When the shared nextest-pair check reads them
    // Then each runner has one default and one --all-features nextest run,
    // and both runs of each pair select the whole workspace
    for (runner, commands) in [("Justfile test recipe", &recipe), ("CI test job", &job)] {
        for run in nextest_pair(commands, runner) {
            assert!(run.contains(&"--workspace"), "{runner}: {run:?}");
        }
    }
    // And the Justfile runs keep the verify-only prefix
    for run in nextest_pair(&recipe, "Justfile test recipe") {
        assert!(run.starts_with(&VERIFY_PREFIX), "{run:?}");
    }
    // And every CI test job cargo run passes --locked
    for command in job
        .iter()
        .filter(|c| is_cargo(c, &["nextest", "run"]) || is_cargo(c, &["test", "--doc"]))
    {
        assert!(command.contains(&"--locked"), "{command:?}");
    }
}

#[test]
fn a_change_to_any_golden_proptest_seed_runs_the_rust_jobs() {
    // Given the CI changes job's rust filter
    let ci = read(".github/workflows/ci.yml");
    let filter = block(&lines(&ci), "            rust:");
    // When a golden test module's proptest seed file is the only change
    // Then a glob over every such seed file matches it
    assert!(
        filter
            .iter()
            .any(|l| l.trim() == "- 'tests/golden/*.proptest-regressions'"),
        "the rust path filter misses tests/golden/*.proptest-regressions"
    );
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

    proptest! {
        /// Removing any set of expected commands from a runner that has them
        /// all makes the check name exactly those commands, in table order.
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
