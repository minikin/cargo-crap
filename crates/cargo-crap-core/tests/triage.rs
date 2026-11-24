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
        "classify",
        "score",
        "diverge",
        ["same", "shape", "obligation", "parameter"],
        "yes",
        "no",
    );
    for id in ["openai", "typesafe"] {
        let wire = provider::by_id(id)
            .unwrap()
            .encode(serde_json::json!({}), "model", &questions);
        let text = wire["questions"].to_string();
        for value in [
            "duplication_kind",
            "worth_extracting",
            "divergence_risk",
            "classify",
            "score",
            "diverge",
            "same",
            "shape",
            "obligation",
            "parameter",
            "yes",
            "no",
        ] {
            assert!(text.contains(value), "{id}: {value}");
        }
    }
}
