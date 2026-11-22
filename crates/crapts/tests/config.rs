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
