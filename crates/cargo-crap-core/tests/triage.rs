use cargo_crap_core::duplicates::triage::{cache, provider, verdict::Kind};

#[test]
fn s36_shared_providers_read_a_verdict_from_the_previous_rust_release() {
    assert_eq!(provider::ids().collect::<Vec<_>>(), ["typesafe", "openai"]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("fb0a17ca1473c168.json"),
        br#"{"kind":"same_logic","confidence":0.8,"worth_extracting":2.0,"divergence_risk":0.3}"#,
    )
    .unwrap();
    let key = cache::CacheKey::new(
        provider::default_provider(),
        "fn a() {}",
        "fn b() {}",
        "jev-latest",
        1,
    );
    let saved = cache::Cache::new(dir.path()).get(key).unwrap();
    assert_eq!(saved.kind, Kind::SameLogic);
}

#[test]
fn a_front_end_can_supply_its_own_content_digest() {
    let key = cache::CacheKey::from_digest([0xab; 32]);
    assert_eq!(key.file_name(), format!("{}.json", "ab".repeat(32)));
}

#[test]
fn a_language_adapter_supplies_text_and_both_providers_emit_the_decodable_ids() {
    use cargo_crap_core::duplicates::triage::provider::QuestionSet;
    let questions = QuestionSet::new(
        "fixture-classify-only",
        "fixture-score-only",
        "fixture-diverge-only",
        [
            "fixture-same-only",
            "fixture-shape-only",
            "fixture-obligation-only",
            "fixture-parameter-only",
        ],
        "fixture-yes-only",
        "fixture-no-only",
    );
    assert_eq!(questions.kind().id(), "duplication_kind");
    assert_eq!(questions.kind().instructions, "fixture-classify-only");
    assert_eq!(
        questions.kind().options().collect::<Vec<_>>(),
        Kind::ALL
            .into_iter()
            .zip([
                "fixture-same-only",
                "fixture-shape-only",
                "fixture-obligation-only",
                "fixture-parameter-only"
            ])
            .collect::<Vec<_>>()
    );
    assert_eq!(questions.worth().id(), "worth_extracting");
    assert_eq!(questions.worth().instructions, "fixture-score-only");
    assert_eq!(
        questions.worth().levels(),
        cargo_crap_core::duplicates::triage::request::WORTH_LEVELS
    );
    assert_eq!(questions.divergence().id(), "divergence_risk");
    assert_eq!(questions.divergence().instructions, "fixture-diverge-only");
    assert_eq!(questions.divergence().if_true, "fixture-yes-only");
    assert_eq!(questions.divergence().if_false, "fixture-no-only");
    for id in ["openai", "typesafe"] {
        let wire = provider::by_id(id)
            .unwrap()
            .encode(serde_json::json!({}), "model", &questions);
        let text = wire["questions"].to_string();
        for value in [
            "duplication_kind",
            "worth_extracting",
            "divergence_risk",
            "fixture-classify-only",
            "fixture-score-only",
            "fixture-diverge-only",
            "fixture-same-only",
            "fixture-shape-only",
            "fixture-obligation-only",
            "fixture-parameter-only",
            "fixture-yes-only",
            "fixture-no-only",
        ] {
            assert!(text.contains(value), "{id}: {value}");
        }
    }
}

#[cfg(feature = "triage")]
#[test]
fn a_budgeted_run_prunes_expired_and_overflow_entries_and_warns_once_on_cleanup_failure() {
    use cargo_crap_core::duplicates::{
        compare::DuplicatePair,
        model::Location,
        triage::{Excerpts, Settings, run_with_cache},
    };
    use std::{
        fs,
        time::{Duration, UNIX_EPOCH},
    };
    let dir = tempfile::tempdir().unwrap();
    let now = UNIX_EPOCH + Duration::from_secs(100);
    let policy = cache::Retention {
        now,
        max_age: Duration::from_secs(30),
        max_entries: 2,
    };
    let key = |n| cache::CacheKey::from_digest([n; 32]);
    for (n, age) in [(1, 3), (2, 2), (3, 1)] {
        fs::File::create(dir.path().join(key(n).file_name()))
            .unwrap()
            .set_modified(now - Duration::from_secs(age))
            .unwrap();
    }
    let at = |name: &str| Location {
        file: "app.ts".into(),
        name: name.into(),
        start_line: 1,
        end_line: 1,
    };
    let pairs = [DuplicatePair {
        first: at("a"),
        second: at("b"),
        score: 1.0,
    }];
    let questions = provider::QuestionSet::new("kind", "worth", "risk", ["rubric"; 4], "yes", "no");
    let mut settings =
        Settings::from_lookup(provider::default_provider(), "fixture", None, |_| None);
    settings.max_requests = Some(0);
    let run = |cache| {
        let mut warnings = Vec::new();
        let result = run_with_cache(
            &pairs,
            &settings,
            &questions,
            Some(&cache),
            |message| warnings.push(message),
            |_| {
                Ok(Excerpts {
                    first: "one".into(),
                    second: "two".into(),
                    locations: None,
                    key: key(4),
                })
            },
        )
        .unwrap();
        assert_eq!(result.skipped, 1);
        warnings
    };
    assert_eq!(
        run(cache::Cache::new(dir.path()).with_retention(policy)),
        Vec::<String>::new()
    );
    assert!(!dir.path().join(key(1).file_name()).exists());
    assert!(dir.path().join(key(2).file_name()).is_file());
    let later = cache::Retention {
        now: now + Duration::from_secs(28),
        ..policy
    };
    assert_eq!(
        run(cache::Cache::new(dir.path()).with_retention(later)),
        Vec::<String>::new()
    );
    let kept = dir.path().join(key(3).file_name());
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>(),
        std::slice::from_ref(&kept)
    );
    let warnings = run(cache::Cache::new(&kept).with_retention(policy));
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with("could not clean the triage cache ("),
        "{warnings:?}"
    );
}
