//! cargo-crap's own [`Tool`] value, the identity its reports carry.
//!
//! It lives outside [`report`](crate::report) on purpose: the report code
//! is shared with other front ends, and `env!("CARGO_PKG_VERSION")` must
//! expand in cargo-crap's package to stamp cargo-crap's version.

use crate::report::{Schemas, Tool};

/// The published HTTPS URL of a schema file in this repository. `concat!`
/// takes only literals, so the base URL is repeated by expansion.
macro_rules! schema_url {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/",
            $file
        )
    };
}

/// cargo-crap's name, version, URLs, PR-comment marker and baseline hint.
pub const CARGO_CRAP: Tool = Tool {
    name: "cargo-crap",
    version: env!("CARGO_PKG_VERSION"),
    info_uri: "https://github.com/minikin/cargo-crap",
    schemas: Schemas {
        report: schema_url!("report-v1.json"),
        delta: schema_url!("delta-v2.json"),
    },
    comment_marker: "<!-- cargo-crap-report -->",
    baseline_hint: "cargo crap --format json",
};

#[cfg(test)]
mod tests {
    use super::CARGO_CRAP;
    use crate::report;

    #[test]
    fn cargo_crap_matches_the_values_the_json_envelope_writes() {
        assert_eq!(CARGO_CRAP.version, report::SCHEMA_VERSION);
        assert_eq!(CARGO_CRAP.schemas.report, report::REPORT_SCHEMA_URL);
        assert_eq!(CARGO_CRAP.schemas.delta, report::DELTA_SCHEMA_URL);
    }
}
