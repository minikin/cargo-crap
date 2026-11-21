//! Runs one case in isolation: a fresh copy of its fixture, a config file
//! the upward walk stops at, and an environment built from an allowlist, so
//! neither the repository nor the host can reach the bytes it records.

use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use assert_cmd::Command;
use tempfile::TempDir;

use crate::encode::{Run, Status};

/// Host variables a case keeps, each only when set: what `cargo metadata`
/// and rustup need to find the toolchain in workspace mode.
const PASSED_THROUGH: [&str; 5] = [
    "PATH",
    "HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
];

/// Every case's environment unless its row overrides an entry.
const DEFAULTS: [(&str, &str); 3] = [("NO_COLOR", "1"), ("COLUMNS", "100"), ("LC_ALL", "C")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// `sample_project`, scored with its `lcov.info` by default.
    P,
    /// `sample_workspace`.
    W,
}

impl Fixture {
    fn dir(self) -> PathBuf {
        let relative = match self {
            Self::P => "sample_project",
            Self::W => "sample_workspace",
        };
        fixtures().join(relative)
    }
}

/// One row of the spec's Cases table.
#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub id: &'static str,
    pub fixture: Fixture,
    pub args: &'static [&'static str],
    /// A file under `golden/configs/`, or `None` for an empty config.
    pub config: Option<&'static str>,
    /// Overrides of the defaults: `Some` sets a variable, `None` removes it.
    pub env: &'static [(&'static str, Option<&'static str>)],
}

/// A case's fixture copy. Dropping it deletes the copy.
pub struct CaseDir {
    dir: TempDir,
}

impl CaseDir {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The argument that stands for the table's "no lcov": a P case without
/// the default `--lcov lcov.info`.
pub const NO_LCOV: &str = "no lcov";

/// A fresh copy of the case's fixture with its config and inputs in place.
pub fn prepare(case: &Case) -> CaseDir {
    prepare_in(case, &std::env::temp_dir())
}

fn prepare_in(
    case: &Case,
    parent: &Path,
) -> CaseDir {
    let dir = tempfile::tempdir_in(parent).expect("temp dir");
    copy_tree(&case.fixture.dir(), dir.path());
    let config = case.config.map_or_else(Vec::new, |name| {
        read(&fixtures().join("golden/configs").join(name))
    });
    write(&dir.path().join(".cargo-crap.toml"), &config);
    for input in case.args.iter().filter_map(|arg| input(arg)) {
        let name = input.file_name().expect("an input file name");
        write(&dir.path().join(name), &read(&input));
    }
    CaseDir { dir }
}

/// An argument that names a file under `golden/lcov/` or `golden/baselines/`.
fn input(arg: &str) -> Option<PathBuf> {
    ["golden/lcov", "golden/baselines"]
        .iter()
        .map(|dir| fixtures().join(dir).join(arg))
        .find(|path| path.is_file())
}

/// Copies the tracked shape of a fixture: build output another test may be
/// writing into the fixture at the same time stays behind. `target/` and
/// `Cargo.lock` are build output only beside a `Cargo.toml`, so a member
/// built on its own loses its own, and a source module named `target` stays.
fn copy_tree(
    from: &Path,
    to: &Path,
) {
    let crate_root = from.join("Cargo.toml").is_file();
    for entry in std::fs::read_dir(from).unwrap_or_else(|e| panic!("{}: {e}", from.display())) {
        let entry = entry.expect("a directory entry");
        let name = entry.file_name();
        if crate_root && (name == "target" || name == "Cargo.lock") {
            continue;
        }
        let (source, dest) = (entry.path(), to.join(&name));
        if entry.file_type().expect("a file type").is_dir() {
            std::fs::create_dir_all(&dest).expect("create dir");
            copy_tree(&source, &dest);
        } else {
            write(&dest, &read(&source));
        }
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn write(
    path: &Path,
    bytes: &[u8],
) {
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}

/// The environment a case's process sees, given the host's variables.
pub fn environment(
    case: &Case,
    copy: &Path,
    host: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = PASSED_THROUGH
        .iter()
        .filter_map(|key| host(key).map(|value| (OsString::from(key), value)))
        .collect();
    let mut set = |key: &str, value: Option<OsString>| {
        env.retain(|(k, _)| k != key);
        if let Some(value) = value {
            env.push((key.into(), value));
        }
    };
    for (key, value) in DEFAULTS {
        set(key, Some(value.into()));
    }
    set("CARGO_TARGET_DIR", Some(copy.join("target").into()));
    for (key, value) in case.env {
        set(key, value.map(OsString::from));
    }
    env
}

/// The case's arguments, with P's default coverage file unless the case
/// names its own or none.
fn arguments(case: &Case) -> Vec<&'static str> {
    if case.args.contains(&NO_LCOV) {
        return case
            .args
            .iter()
            .copied()
            .filter(|a| *a != NO_LCOV)
            .collect();
    }
    let default = case.fixture == Fixture::P && !case.args.contains(&"--lcov");
    let lcov: &[&str] = if default {
        &["--lcov", "lcov.info"]
    } else {
        &[]
    };
    lcov.iter().chain(case.args).copied().collect()
}

pub fn run_with(
    case: &Case,
    copy: &CaseDir,
    host: &dyn Fn(&str) -> Option<OsString>,
) -> Run {
    let output = Command::cargo_bin("cargo-crap")
        .expect("the cargo-crap binary")
        .current_dir(copy.root())
        .env_clear()
        .envs(environment(case, copy.root(), host))
        .args(arguments(case))
        .output()
        .unwrap_or_else(|e| panic!("{}: running cargo-crap: {e}", case.id));
    Run {
        stdout: output.stdout,
        stderr: output.stderr,
        status: status_of(output.status),
    }
}

/// Runs the case on a fresh copy, with this process's environment as host.
pub fn run(case: &Case) -> Run {
    run_with(case, &prepare(case), &|key| std::env::var_os(key))
}

fn status_of(status: ExitStatus) -> Status {
    match (status.code(), status.signal()) {
        (Some(code), _) => Status::Exit(code),
        (None, Some(signal)) => Status::Signal(signal),
        (None, None) => panic!("a finished process with neither exit code nor signal: {status:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::encode;

    const P_JSON: Case = Case {
        id: "probe_p_json",
        fixture: Fixture::P,
        args: &["--format", "json"],
        config: None,
        env: &[],
    };

    const P_MARKDOWN: Case = Case {
        id: "probe_p_markdown",
        fixture: Fixture::P,
        args: &["--format", "markdown"],
        config: None,
        env: &[],
    };

    const P_COLOR: Case = Case {
        id: "probe_p_color",
        fixture: Fixture::P,
        args: &[],
        config: None,
        env: &[("FORCE_COLOR", Some("1")), ("NO_COLOR", None)],
    };

    const W_HUMAN: Case = Case {
        id: "probe_w_human",
        fixture: Fixture::W,
        args: &["--workspace"],
        config: None,
        env: &[],
    };

    const HOSTILE: [(&str, &str); 9] = [
        ("GITHUB_SERVER_URL", "https://evil.example"),
        ("GITHUB_REPOSITORY", "evil/repo"),
        ("GITHUB_SHA", "deadbeef"),
        ("NO_COLOR", ""),
        ("FORCE_COLOR", "1"),
        ("CLICOLOR_FORCE", "1"),
        ("COLUMNS", "31"),
        ("LANG", "de_DE.UTF-8"),
        ("TYPESAFE_API_KEY", "not-a-key"),
    ];

    fn clean_host(key: &str) -> Option<OsString> {
        PASSED_THROUGH
            .contains(&key)
            .then(|| std::env::var_os(key))
            .flatten()
    }

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).expect("utf-8 output")
    }

    const OK: Status = Status::Exit(0);

    #[test]
    fn the_repositorys_own_config_does_not_reach_a_case() {
        // Given the repository root holds a .cargo-crap.toml with [signals] enabled = true
        let own =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cargo-crap.toml"))
                .expect("the repository's config");
        assert!(own.contains("[signals]\nenabled = true"));
        // And a case whose Config column is "-", copied below a directory holding that config
        let parent = tempfile::tempdir().expect("parent");
        write(&parent.path().join(".cargo-crap.toml"), own.as_bytes());
        let copy = prepare_in(&P_JSON, parent.path());
        // When the case runs
        let run = run_with(&P_JSON, &copy, &clean_host);
        // Then the case's copy holds an empty .cargo-crap.toml, where the upward walk stops
        assert_eq!(read(&copy.root().join(".cargo-crap.toml")), b"");
        // And its JSON output carries no "signals" field
        assert_eq!(run.status, OK, "{}", text(&run.stderr));
        assert!(text(&run.stdout).contains("\"entries\""));
        assert!(!text(&run.stdout).contains("\"signals\""));
    }

    #[test]
    fn without_its_empty_config_the_copy_would_read_the_parents() {
        let parent = tempfile::tempdir().expect("parent");
        write(
            &parent.path().join(".cargo-crap.toml"),
            b"[signals]\nenabled = true\n",
        );
        let copy = prepare_in(&P_JSON, parent.path());
        std::fs::remove_file(copy.root().join(".cargo-crap.toml")).expect("remove config");
        let run = run_with(&P_JSON, &copy, &clean_host);
        assert!(text(&run.stdout).contains("\"signals\""));
    }

    #[test]
    fn a_named_config_is_written_as_the_copys_config() {
        let case = Case {
            config: Some("signals.toml"),
            ..P_JSON
        };
        let copy = prepare(&case);
        assert_eq!(
            read(&copy.root().join(".cargo-crap.toml")),
            read(&fixtures().join("golden/configs/signals.toml"))
        );
    }

    /// Where the re-run test binary writes each probe's document.
    const HOSTILE_OUT: &str = "GOLDEN_HOSTILE_OUT";

    const PROBES: [Case; 4] = [P_JSON, P_MARKDOWN, P_COLOR, W_HUMAN];

    #[test]
    fn the_hosts_environment_does_not_reach_a_case() {
        // Given GITHUB_*, colour, COLUMNS, LANG, target-dir and API-key variables
        // set to unusual values in the environment of the process that runs the cases
        let out = tempfile::tempdir().expect("out");
        let shared_target = tempfile::tempdir().expect("shared target");
        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "run::tests::probes_under_a_hostile_host",
                "--exact",
                "--ignored",
            ])
            .envs(HOSTILE)
            .env("CARGO_TARGET_DIR", shared_target.path())
            .env("CARGO_BUILD_TARGET_DIR", shared_target.path())
            .env(HOSTILE_OUT, out.path())
            .output()
            .expect("re-running the test binary");
        assert!(status.status.success(), "{}", text(&status.stdout));
        assert_eq!(
            std::fs::read_dir(shared_target.path())
                .expect("shared target")
                .count(),
            0,
            "a case wrote into the host's target directory"
        );
        // When the cases run
        for case in PROBES {
            let clean = run_with(&case, &prepare(&case), &clean_host);
            assert_eq!(clean.status, OK, "{}: {}", case.id, text(&clean.stderr));
            // Then every case's output equals the output under a clean host
            let hostile = std::fs::read_to_string(out.path().join(case.id)).expect("probe output");
            assert_eq!(
                hostile,
                encode(case.id, &clean),
                "{} saw the host's environment",
                case.id
            );
        }
    }

    /// Runs only inside `the_hosts_environment_does_not_reach_a_case`, which
    /// sets the hostile variables in this process's real environment.
    #[test]
    #[ignore = "run by the_hosts_environment_does_not_reach_a_case"]
    fn probes_under_a_hostile_host() {
        let Some(out) = std::env::var_os(HOSTILE_OUT) else {
            return;
        };
        for case in PROBES {
            let copy = prepare(&case);
            let run = run_with(&case, &copy, &|key| std::env::var_os(key));
            write(
                &Path::new(&out).join(case.id),
                encode(case.id, &run).as_bytes(),
            );
            if case.fixture == Fixture::W {
                assert!(
                    copy.root().join("target/cargo-crap").is_dir(),
                    "the cache left the copy"
                );
            }
        }
    }

    #[test]
    fn a_case_sees_only_the_allowlist_and_its_own_env() {
        // Then a case's process sees only the passed-through variables, LC_ALL=C,
        // CARGO_TARGET_DIR inside its own copy, and the case's Env column
        let host = |key: &str| {
            let value = match key {
                "PATH" => "/host/bin",
                "HOME" => "/host/home",
                "CARGO_HOME" => "/host/cargo",
                "RUSTUP_HOME" => "/host/rustup",
                "GITHUB_SHA" => "deadbeef",
                "NO_COLOR" => "1",
                _ => return None,
            };
            Some(OsString::from(value))
        };
        let mut env = environment(&P_COLOR, Path::new("/copy"), &host);
        env.sort();
        let expected: Vec<(OsString, OsString)> = [
            ("CARGO_HOME", "/host/cargo"),
            ("CARGO_TARGET_DIR", "/copy/target"),
            ("COLUMNS", "100"),
            ("FORCE_COLOR", "1"),
            ("HOME", "/host/home"),
            ("LC_ALL", "C"),
            ("PATH", "/host/bin"),
            ("RUSTUP_HOME", "/host/rustup"),
        ]
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
        // RUSTUP_TOOLCHAIN is unset on this host, so the case has none
        assert_eq!(env, expected);
        let with_toolchain = |key: &str| {
            (key == "RUSTUP_TOOLCHAIN")
                .then(|| OsString::from("1.99.0"))
                .or_else(|| host(key))
        };
        let env = environment(&P_JSON, Path::new("/copy"), &with_toolchain);
        assert!(env.contains(&("RUSTUP_TOOLCHAIN".into(), "1.99.0".into())));
    }

    #[test]
    fn an_input_named_by_an_argument_is_copied_by_file_name() {
        let case = Case {
            args: &["--lcov", "p_other.info"],
            ..P_JSON
        };
        let copy = prepare(&case);
        assert_eq!(
            read(&copy.root().join("p_other.info")),
            read(&fixtures().join("golden/lcov/p_other.info"))
        );
        let run = run_with(&case, &copy, &clean_host);
        assert_eq!(run.status, OK, "{}", text(&run.stderr));
        assert!(
            text(&run.stderr).contains("src/other.rs"),
            "{}",
            text(&run.stderr)
        );
    }

    #[test]
    fn a_colour_case_emits_escape_codes() {
        let run = run_with(&P_COLOR, &prepare(&P_COLOR), &clean_host);
        assert!(
            run.stdout.contains(&0x1b),
            "no ESC in {}",
            text(&run.stdout)
        );
    }

    #[test]
    fn a_workspace_case_keeps_its_cache_inside_its_copy() {
        // A Cargo config above the copy that sends build output elsewhere:
        // the case's own CARGO_TARGET_DIR must win over it.
        let parent = tempfile::tempdir().expect("parent");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        std::fs::create_dir(parent.path().join(".cargo")).expect("mkdir .cargo");
        let config = format!("[build]\ntarget-dir = {:?}\n", elsewhere.path());
        write(&parent.path().join(".cargo/config.toml"), config.as_bytes());
        let copy = prepare_in(&W_HUMAN, parent.path());
        let run = run_with(&W_HUMAN, &copy, &clean_host);
        assert_eq!(run.status, OK, "{}", text(&run.stderr));
        assert!(copy.root().join("target/cargo-crap").is_dir());
        assert_eq!(
            std::fs::read_dir(elsewhere.path())
                .expect("elsewhere")
                .count(),
            0,
            "the cache followed the Cargo config out of the copy"
        );
    }

    #[test]
    fn p_cases_get_the_default_coverage_file_unless_they_name_one() {
        assert_eq!(
            arguments(&P_JSON),
            ["--lcov", "lcov.info", "--format", "json"]
        );
        let own = Case {
            args: &["--lcov", "p_other.info"],
            ..P_JSON
        };
        assert_eq!(arguments(&own), ["--lcov", "p_other.info"]);
        let none = Case {
            args: &[NO_LCOV, "--summary"],
            ..P_JSON
        };
        assert_eq!(arguments(&none), ["--summary"]);
        assert_eq!(arguments(&W_HUMAN), ["--workspace"]);
    }

    #[test]
    fn a_copy_leaves_build_output_behind() {
        let source = tempfile::tempdir().expect("source");
        let s = source.path();
        for (file, bytes) in [
            ("Cargo.toml", "[workspace]\n"),
            ("Cargo.lock", "lock"),
            ("target/debug/out", "build"),
            ("crates/a/src/lib.rs", "pub fn a() {}\n"),
            ("crates/a/target/x", "build"),
            ("crates/a/Cargo.lock", "lock"),
            ("crates/a/Cargo.toml", "[package]\n"),
            ("crates/a/src/target/mod.rs", "pub mod nested;\n"),
        ] {
            std::fs::create_dir_all(s.join(file).parent().expect("parent")).expect("mkdir");
            write(&s.join(file), bytes.as_bytes());
        }
        let dest = tempfile::tempdir().expect("dest");
        copy_tree(s, dest.path());
        let d = dest.path();
        assert_eq!(read(&d.join("crates/a/src/lib.rs")), b"pub fn a() {}\n");
        assert!(d.join("Cargo.toml").is_file());
        assert_eq!(
            read(&d.join("crates/a/src/target/mod.rs")),
            b"pub mod nested;\n"
        );
        for gone in [
            "Cargo.lock",
            "target",
            "crates/a/target",
            "crates/a/Cargo.lock",
        ] {
            assert!(!d.join(gone).exists(), "{gone} was copied");
        }
    }

    #[test]
    fn a_signal_is_recorded_with_its_number() {
        assert_eq!(status_of(ExitStatus::from_raw(9)), Status::Signal(9));
        assert_eq!(status_of(ExitStatus::from_raw(2 << 8)), Status::Exit(2));
    }
}
