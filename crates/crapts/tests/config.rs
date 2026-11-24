use crapts::config::{Config, load};
use std::{fs, process::Command};

#[test]
fn worktree_scopes_the_nearest_file_including_its_root() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let pkg = repo.join("packages");
    let cwd = pkg.join("app");
    fs::create_dir_all(&cwd).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    fs::write(temp.path().join(".crapts.toml"), "threshold=39").unwrap();
    assert_eq!(load(&cwd).unwrap(), (None, Config::default()));
    let root_config = repo.join(".crapts.toml");
    fs::write(&root_config, "threshold=20").unwrap();
    let (path, settings) = load(&cwd).unwrap();
    assert_eq!(path, Some(root_config.canonicalize().unwrap()));
    assert_eq!(settings.threshold, Some(20.0));
    let nearest = pkg.join(".crapts.toml");
    fs::write(&nearest, "sort='file'").unwrap();
    let (path, settings) = load(&cwd).unwrap();
    assert_eq!(path, Some(nearest.canonicalize().unwrap()));
    assert_eq!(settings.threshold, None);
    fs::write(&nearest, [0xff]).unwrap();
    assert_eq!(
        load(&cwd).unwrap_err().path(),
        nearest.canonicalize().unwrap()
    );
    assert!(load(&cwd.join("absent")).is_err());
}

#[cfg(unix)]
#[test]
fn git_and_the_filesystem_can_spell_the_same_worktree_boundary_differently() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let key = "CRAPTS_TEST_CONFIG_BOUNDARY";
    if let Some(dir) = std::env::var_os(key) {
        let dir = std::path::PathBuf::from(dir);
        let (path, _) = load(&dir.join("repo/pkg")).unwrap();
        assert_eq!(
            path,
            Some(dir.join("repo/.crapts.toml").canonicalize().unwrap())
        );
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path();
    fs::create_dir_all(dir.join("repo/pkg")).unwrap();
    fs::write(dir.join("repo/.crapts.toml"), "").unwrap();
    symlink(dir.join("repo"), dir.join("alias")).unwrap();
    let git = dir.join("git");
    fs::write(&git,"#!/bin/sh\ncase \"$2\" in\n--show-toplevel) echo \"$CRAPTS_TEST_CONFIG_BOUNDARY/alias\" ;;\n*) echo true; echo pkg/ ;;\nesac\n").unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "git_and_the_filesystem_can_spell_the_same_worktree_boundary_differently",
            "--nocapture",
        ])
        .env(key, dir)
        .env("PATH", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn git_free_lookup_stops_at_a_marker_and_walks_past_its_former_location() {
    let key = "CRAPTS_TEST_CONFIG_NO_GIT";
    if let Some(root) = std::env::var_os(key) {
        let root = std::path::PathBuf::from(root);
        let repo = root.join("repo");
        let cwd = repo.join("pkg");
        fs::create_dir_all(&cwd).unwrap();
        let outer = root.join(".crapts.toml");
        fs::write(&outer, "threshold=44").unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        assert_eq!(load(&cwd).unwrap(), (None, Config::default()));
        fs::remove_dir(repo.join(".git")).unwrap();
        let (path, cfg) = load(&cwd).unwrap();
        assert_eq!(path, Some(outer.canonicalize().unwrap()));
        assert_eq!(cfg.threshold, Some(44.));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "git_free_lookup_stops_at_a_marker_and_walks_past_its_former_location",
            "--nocapture",
        ])
        .env(key, root.path())
        .env("PATH", root.path().join("no-git"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn signal_config(cwd: &std::path::Path) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_crapts"))
        .current_dir(cwd)
        .args([
            "--print-config",
            "--coverage",
            "missing",
            "--baseline",
            "absent",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let raw: String = std::str::from_utf8(&out.stdout)
        .unwrap()
        .split_whitespace()
        .collect();
    let limits = raw.split_once("\"limits\":{").unwrap().1;
    let mut keys = limits.match_indices("\":{\"value\":");
    for name in ["nesting", "lines", "bumps", "bool_ops"] {
        let (index, _) = keys.next().unwrap();
        assert!(limits[..index].ends_with(name), "{limits}");
    }
    for name in ["enabled", "gate"] {
        assert!(raw.contains(&format!("\"{name}\":{{\"value\":")), "{raw}");
    }
    serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["settings"]["signals"].clone()
}

#[test]
fn s14_print_each_signal_default_without_scoring_inputs() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let unset = json!({"value":null,"origin":"default"});
    assert_eq!(
        signal_config(dir.path()),
        json!({
            "enabled":{"value":false,"origin":"default"},
            "gate":{"value":["nesting","bumps","bool_ops","stubs"],"origin":"default"},
            "limits":{"nesting":unset,"lines":unset,"bumps":unset,"bool_ops":unset}
        })
    );
}

#[test]
fn s15_configured_leaves_keep_their_origin_and_canonical_names() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("sub");
    fs::create_dir(&nested).unwrap();
    let path = dir.path().join(".crapts.toml");
    let input = "[signals]\nenabled=false\ngate=['stubs','bool_ops','bool-ops']\n[signals.limits]\nlines=0\nbool-ops=4294967295\n";
    for spelling in ["bool-ops=", "bool_ops="] {
        fs::write(&path, input.replace("bool-ops=", spelling)).unwrap();
        let value = signal_config(&nested);
        assert_eq!(
            value,
            json!({
               "enabled":{"value":false,"origin":"config"},
               "gate":{"value":["bool_ops","stubs"],"origin":"config"},
               "limits":{"nesting":{"value":null,"origin":"default"},"lines":{"value":0,"origin":"config"},
                         "bumps":{"value":null,"origin":"default"},"bool_ops":{"value":u32::MAX,"origin":"config"}}
            })
        );
    }
    fs::write(
        &path,
        "[signals]\nenabled=true\ngate=[]\n[signals.limits]\nnesting=1\nbumps=3",
    )
    .unwrap();
    let value = signal_config(&nested);
    for (pointer, expected) in [
        ("/enabled", json!(true)),
        ("/gate", json!([])),
        ("/limits/nesting", json!(1)),
        ("/limits/bumps", json!(3)),
    ] {
        assert_eq!(
            value.pointer(pointer),
            Some(&json!({"value":expected,"origin":"config"}))
        );
    }
}

#[test]
fn s16_invalid_signal_config_precedes_scoring_even_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    fs::write(cwd.join("f.ts"), "function f() {}").unwrap();
    fs::write(cwd.join("lcov.info"), "SF:f.ts\nDA:1,1\nend_of_record\n").unwrap();
    fs::write(cwd.join("report"), "keep").unwrap();
    for (bad, details) in [
        (
            "gate=['nestng']",
            vec!["nestng", "nesting, lines, bumps, bool-ops, stubs, panics"],
        ),
        ("gate='stubs'", vec!["gate"]),
        ("extra=true", vec!["extra"]),
        (
            "[signals.limits]\nnesting=-1",
            vec!["signals.limits.nesting", "-1"],
        ),
        (
            "[signals.limits]\nlines=4294967296",
            vec!["signals.limits.lines", "4294967296"],
        ),
        ("[signals.limits]\nbumps=1.5", vec!["bumps"]),
        ("[signals.limits]\nstubs=0", vec!["unknown key stubs"]),
        ("[signals.limits]\npanics=1", vec!["unknown key panics"]),
        (
            "[signals.limits]\nbool-ops=1\nbool_ops=1",
            vec!["duplicate", "bool-ops"],
        ),
    ] {
        fs::write(
            cwd.join(".crapts.toml"),
            format!("[signals]\nenabled=false\n{bad}"),
        )
        .unwrap();
        for args in [
            vec!["--print-config"],
            vec!["--coverage", "lcov.info", "--output", "report"],
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_crapts"))
                .current_dir(cwd)
                .args(args)
                .output()
                .unwrap();
            let err = String::from_utf8(out.stderr).unwrap();
            assert_eq!(out.status.code(), Some(2), "{bad}: {err}");
            assert_eq!(out.stdout, [] as [u8; 0]);
            assert!(err.contains(".crapts.toml"), "{err}");
            for detail in &details {
                assert!(err.contains(detail), "{bad}: {err}");
            }
            assert_eq!(fs::read(cwd.join("report")).unwrap(), b"keep");
        }
    }
    for flag in ["--help", "--version"] {
        assert!(
            Command::new(env!("CARGO_BIN_EXE_crapts"))
                .current_dir(cwd)
                .arg(flag)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
