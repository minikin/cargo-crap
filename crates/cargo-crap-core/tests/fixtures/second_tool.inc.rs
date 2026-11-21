Tool {
    policy: CARGO_CRAP_FIXTURE.policy,
    name: "other-tool",
    version: "7.7.7-other",
    info_uri: "https://example.test/other-tool",
    schemas: Schemas {
        report: "https://example.test/schemas/other-report.json",
        delta: "https://example.test/schemas/other-delta.json",
    },
    comment_marker: "<!-- other-tool-marker -->",
    baseline_hint: "other-tool --json",
}
