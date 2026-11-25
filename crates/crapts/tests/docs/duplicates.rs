#[cfg(feature = "triage")]
#[expect(
    dead_code,
    reason = "the docs replay only needs the recording provider's request count"
)]
#[path = "../../../../tests/support/api_stub.rs"]
mod docs_api;

use crate::{common, fixture, pages};
use pages::Page;
use std::fmt::Write as _;
use std::fs;

const GUIDE: &str = "docs/crapts/guides/duplicates.md";

fn command(
    page: &Page,
    args: &[&str],
) -> Vec<String> {
    pages::command_lines(page)
        .into_iter()
        .filter_map(|(_, line)| pages::crapts_arguments(&line).unwrap())
        .find(|words| words == args)
        .unwrap_or_else(|| panic!("{} lacks crapts {}", page.path, args.join(" ")))
}

fn revision(
    repo: &fixture::Repo,
    names: &[&str],
) {
    let mut lcov = String::new();
    for name in names {
        let path = format!("{name}.ts");
        fs::write(repo.root().join(&path), format!("export function {name}(xs: number[]) {{ let total=0; for (const x of xs) {{ if (x>0) total+=2*x; else total-=x; }} return total; }}\n")).unwrap();
        writeln!(lcov, "SF:{path}\nDA:1,1\nend_of_record").unwrap();
    }
    fs::create_dir_all(repo.root().join("coverage")).unwrap();
    fs::write(repo.root().join("coverage/lcov.info"), lcov).unwrap();
    repo.git(&["add", "*.ts", "coverage/lcov.info"]);
    repo.git(&["commit", "-qm", "update copies and coverage"]);
}

#[test]
fn s38_the_guide_replays_a_first_look_and_measures_the_grown_group() {
    let page = Page::load(GUIDE);
    assert!(
        Page::load("README.md")
            .text
            .lines()
            .any(|s| s == "| Duplicates and triage | yes | yes |")
    );
    assert!(crate::index::linked_files(&Page::load(crate::index::INDEX)).contains(GUIDE));
    let repo = fixture::Repo::new();
    revision(&repo, &["north", "south"]);
    let look = repo.crapts(&command(&page, &["--duplicates"]));
    assert_eq!(look.code, Some(0));
    assert!(look.stderr.contains("No coverage supplied."));
    for word in ["north", "south", "1 duplicate candidate"] {
        assert!(look.stdout.contains(word), "{word}: {}", look.stdout);
    }
    let record = command(
        &page,
        &[
            "--duplicates",
            "--coverage",
            "coverage/lcov.info",
            "--format",
            "json",
            "--all-rows",
            "--output",
            "crapts-baseline.json",
        ],
    );
    let baseline = repo.crapts(&record);
    assert_eq!(baseline.code, Some(0), "{}", baseline.stderr);
    let before =
        common::validated_report(&fs::read(repo.root().join("crapts-baseline.json")).unwrap());
    assert_eq!(
        before["duplicate_groups"][0]["members"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    repo.git(&["checkout", "-qb", "feature"]);
    revision(&repo, &["north", "south", "west"]);
    let args = command(
        &page,
        &[
            "--duplicates",
            "--coverage",
            "coverage/lcov.info",
            "--baseline",
            "crapts-baseline.json",
            "--format",
            "json",
        ],
    );
    let delta = repo.crapts(&args);
    assert_eq!(delta.code, Some(0), "{}", delta.stderr);
    let report = common::validated_delta(delta.stdout.as_bytes());
    assert_eq!(report["duplicate_groups"][0]["grown_from"], 2);
    assert_eq!(report["duplicate_groups"][0]["status"], "grown");
    let changed = command(
        &page,
        &[
            "--duplicates",
            "--coverage",
            "coverage/lcov.info",
            "--base",
            "main",
            "--format",
            "json",
        ],
    );
    let run = repo.crapts(&changed);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let report = common::validated_report(run.stdout.as_bytes());
    assert_eq!(
        report["duplicate_groups"][0]["members"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(report["changed"]["counts"]["touched"], 1);
}

#[cfg(feature = "triage")]
#[test]
fn s38_the_documented_cold_budget_keeps_the_provider_offline() {
    let page = Page::load(GUIDE);
    let block = page
        .blocks()
        .into_iter()
        .find(|block| block.info == "toml" && block.body.contains("max-requests = 0"))
        .expect("zero-budget TOML on the guide");
    let repo = fixture::Repo::new();
    revision(&repo, &["north", "south"]);
    fs::write(repo.root().join(".crapts.toml"), block.body).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let stub = docs_api::ApiStub::scripted(vec![docs_api::Reply::status(403)]);
    let endpoint = stub.base_url();
    let args = command(&page, &["--duplicates", "--format", "json"]);
    let out = common::binary(&repo.root())
        .env("TYPESAFE_API_KEY", "docs-fixture")
        .env("TYPESAFE_BASE_URL", &endpoint)
        .env("OPENAI_BASE_URL", &endpoint)
        .env_remove("OPENAI_API_KEY")
        .env("HOME", cache.path())
        .env("XDG_CACHE_HOME", cache.path())
        .env("LOCALAPPDATA", cache.path())
        .args(&args)
        .output()
        .unwrap();
    common::check(&out, 0);
    let report = common::validated_report(&out.stdout);
    assert_eq!(report["untriaged_groups"], 1);
    assert!(report["duplicate_groups"][0].get("triage").is_none());
    assert_eq!(stub.request_count(), 0);
}
