use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs")
}

pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn from_output(out: &Output) -> Run {
        Run {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}

/// A temporary Git repository that no developer configuration reaches.
pub struct Repo {
    dir: TempDir,
    empty_config: PathBuf,
}

impl Repo {
    pub fn new() -> Repo {
        Repo::in_dir(tempfile::tempdir().unwrap())
    }

    /// A shared repository lives in a static and is never dropped, so it is
    /// kept under the target directory and copies older than a day go.
    fn shared() -> Repo {
        let parent = Path::new(env!("CARGO_TARGET_TMPDIR")).join("crapts-docs");
        fs::create_dir_all(&parent).unwrap();
        let day_ago = SystemTime::now() - Duration::from_hours(24);
        for entry in fs::read_dir(&parent).unwrap().flatten() {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified < day_ago);
            if stale {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        Repo::in_dir(tempfile::tempdir_in(parent).unwrap())
    }

    fn in_dir(dir: TempDir) -> Repo {
        let empty_config = dir.path().join(".git-empty-config");
        fs::write(&empty_config, "").unwrap();
        fs::create_dir(dir.path().join("home")).unwrap();
        fs::create_dir(dir.path().join("repo")).unwrap();
        let repo = Repo { dir, empty_config };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    pub fn root(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn isolate(
        &self,
        cmd: &mut Command,
    ) {
        let home = self.dir.path().join("home");
        cmd.current_dir(self.root())
            .env("GIT_CONFIG_GLOBAL", &self.empty_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &home);
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
        ] {
            cmd.env_remove(key);
        }
    }

    pub fn git(
        &self,
        args: &[&str],
    ) -> String {
        let mut cmd = Command::new("git");
        self.isolate(&mut cmd);
        for (key, value) in [
            ("GIT_AUTHOR_NAME", "fixture"),
            ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
            ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
            ("GIT_COMMITTER_NAME", "fixture"),
            ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
            ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
        ] {
            cmd.env(key, value);
        }
        let out = cmd.args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn crapts<S: AsRef<std::ffi::OsStr>>(
        &self,
        args: &[S],
    ) -> Run {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_crapts"));
        self.isolate(&mut cmd);
        cmd.env("NO_COLOR", "1");
        for key in [
            "FORCE_COLOR",
            "CLICOLOR_FORCE",
            "COLUMNS",
            "GITHUB_SERVER_URL",
            "GITHUB_REPOSITORY",
            "GITHUB_SHA",
        ] {
            cmd.env_remove(key);
        }
        Run::from_output(&cmd.args(args).output().unwrap())
    }

    /// Line endings are pinned to LF so commit ids match on every platform.
    pub fn commit_tree(
        &self,
        tree: &Path,
        message: &str,
    ) {
        let root = self.root();
        for entry in fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n != ".git") {
                remove(&path);
            }
        }
        copy_tree(tree, &root);
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--message", message]);
    }

    pub fn copy(
        &self,
        from: &str,
        to: &str,
    ) {
        fs::copy(self.root().join(from), self.root().join(to)).unwrap();
    }
}

fn remove(path: &Path) {
    if path.is_dir() {
        fs::remove_dir_all(path).unwrap();
    } else {
        fs::remove_file(path).unwrap();
    }
}

fn copy_tree(
    from: &Path,
    to: &Path,
) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_tree(&path, &target);
        } else {
            let text = fs::read_to_string(&path).unwrap();
            fs::write(&target, text.replace("\r\n", "\n")).unwrap();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeatureCoverage {
    Own,
    KeptFromMain,
}

pub fn cart() -> &'static Repo {
    static CART: OnceLock<Repo> = OnceLock::new();
    CART.get_or_init(|| {
        prepare(Repo::shared(), FeatureCoverage::Own).unwrap_or_else(|e| panic!("{e}"))
    })
}

pub fn covered_wrapper() -> Repo {
    use std::fmt::Write as _;
    let repo = Repo::new();
    let original = crate::pages::repo_root().join("npm/crapts");
    for file in [".crapts.toml", "package.json", "README.md"] {
        fs::copy(original.join(file), repo.root().join(file)).unwrap();
    }
    for dir in ["bin", "lib", "test"] {
        let target = repo.root().join(dir);
        fs::create_dir_all(&target).unwrap();
        copy_tree(&original.join(dir), &target);
    }
    let source = fs::read_to_string(repo.root().join("lib/resolve.js")).unwrap();
    let mut cov = "SF:lib/resolve.js\n".to_owned();
    for n in 1..=source.lines().count() {
        writeln!(cov, "DA:{n},1").unwrap();
    }
    cov.push_str("end_of_record\n");
    fs::write(repo.root().join("wrapper.info"), cov).unwrap();
    repo
}

pub fn signal_change() -> &'static Repo {
    static SIGNAL_CHANGE: OnceLock<Repo> = OnceLock::new();
    SIGNAL_CHANGE.get_or_init(|| {
        let repo = Repo::shared();
        let root = repo.root();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(
            root.join(".gitignore"),
            "signals-baseline.json\nsignals.info\n",
        )
        .unwrap();
        fs::write(root.join(".crapts.toml"), "[signals]\nenabled = true\n").unwrap();
        let source = "export function f() {\n  let x = 0\n  x += 1\n  return x\n}\n";
        fs::write(root.join("src/f.ts"), source).unwrap();
        repo.git(&["add", "--all"]);
        repo.git(&["commit", "--quiet", "--message", "measured base"]);
        fs::write(
            root.join("signals.info"),
            "SF:src/f.ts\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,1\nDA:5,1\nend_of_record\n",
        )
        .unwrap();
        let baseline = repo.crapts(&[
            "--coverage",
            "signals.info",
            "--format",
            "json",
            "--all-rows",
            "--output",
            "signals-baseline.json",
        ]);
        expect_exit(&baseline, 0, "the measured baseline").unwrap();
        fs::write(
            root.join("src/f.ts"),
            source.replace("return x", "debugger; return x"),
        )
        .unwrap();
        repo
    })
}

pub fn cart_with(coverage: FeatureCoverage) -> Result<Repo, String> {
    prepare(Repo::new(), coverage)
}

fn prepare(
    repo: Repo,
    coverage: FeatureCoverage,
) -> Result<Repo, String> {
    repo.commit_tree(&fixtures().join("base"), "base");
    repo.git(&["checkout", "--quiet", "-b", "feature"]);
    repo.commit_tree(&fixtures().join("head"), "head");
    repo.git(&["checkout", "--quiet", "main"]);
    repo.copy("coverage/lcov.data", "coverage/lcov.info");
    let run = repo.crapts(&[
        "--coverage",
        "coverage/lcov.info",
        "--format",
        "json",
        "--all-rows",
        "--output",
        "crapts-baseline.json",
    ]);
    expect_exit(&run, 0, "the main baseline run")?;
    let baseline = fs::read_to_string(repo.root().join("crapts-baseline.json")).unwrap();
    expect_scores(
        "main",
        &baseline,
        &[("checkout", 3, 12.0), ("total", 1, 1.0)],
    )?;
    repo.git(&["checkout", "--quiet", "feature"]);
    if coverage == FeatureCoverage::Own {
        repo.copy("coverage/lcov.data", "coverage/lcov.info");
    }
    for file in ["coverage/lcov.info", "coverage/coverage-final.json"] {
        let run = repo.crapts(&["--coverage", file, "--format", "json"]);
        expect_exit(&run, 0, file)?;
        let label = format!("feature with {file}");
        expect_scores(
            &label,
            &run.stdout,
            &[("checkout", 6, 42.0), ("total", 1, 1.0)],
        )?;
    }
    Ok(repo)
}

fn expect_exit(
    run: &Run,
    code: i32,
    what: &str,
) -> Result<(), String> {
    if run.code == Some(code) {
        Ok(())
    } else {
        Err(format!(
            "{what} exited {:?}, expected {code}: {}",
            run.code, run.stderr
        ))
    }
}

pub fn expect_scores(
    label: &str,
    report: &str,
    expected: &[(&str, u64, f64)],
) -> Result<(), String> {
    let report: Value = serde_json::from_str(report).map_err(|e| format!("{label}: {e}"))?;
    for (function, cc, crap) in expected {
        let entry = report["entries"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["function"] == *function))
            .ok_or_else(|| format!("{label}: no row for {function}"))?;
        let actual = (entry["cyclomatic"].as_u64(), entry["crap"].as_f64());
        if actual != (Some(*cc), Some(*crap)) {
            return Err(format!(
                "{label}: {function} scores CC {:?} CRAP {:?}, expected CC {cc} CRAP {crap}",
                actual.0, actual.1
            ));
        }
    }
    Ok(())
}

pub const COMPLEXITY_PAGE: &str = "docs/crapts/reference/complexity.md";

pub fn complexity() -> &'static Repo {
    static COMPLEXITY: OnceLock<Repo> = OnceLock::new();
    COMPLEXITY.get_or_init(|| {
        let repo = Repo::shared();
        repo.commit_tree(&fixtures().join("complexity"), "complexity");
        repo.copy("coverage/lcov.data", "coverage/lcov.info");
        repo
    })
}

/// The complexity reference runs in its own repository so its example
/// leaves the cart scores and samples unchanged.
pub fn repo_for_page(path: &str) -> &'static Repo {
    if path == COMPLEXITY_PAGE {
        complexity()
    } else {
        cart()
    }
}
