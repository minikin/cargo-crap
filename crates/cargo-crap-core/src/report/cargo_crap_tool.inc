Tool {
    policy: Policy {
        envelope: EnvelopeKind::CargoCrapV1,
        pairing: PairPolicy::FileFunctionLine,
        fail_above_counts: Population::Shown,
        summary_formats: &[
            Format::Human,
            Format::Markdown,
            Format::PrComment,
            Format::Sarif,
            Format::Shields,
        ],
        signal_detail_hint: Some("run without --summary for the reasons"),
        signal_slice_hint: Some("use --format json with the top and min limits removed for the reasons"),
        tie_order: TieOrder::Stable,
        notice_spacing: NoticeSpacing::Separated,
    },
    name: "cargo-crap",
    version: "0.0.0-test",
    info_uri: "https://github.com/minikin/cargo-crap",
    schemas: Schemas {
        report: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/report-v1.json",
        delta: "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/delta-v2.json",
    },
    comment_marker: "<!-- cargo-crap-report -->",
    baseline_hint: "cargo crap --format json",
}
