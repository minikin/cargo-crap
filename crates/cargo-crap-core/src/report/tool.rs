//! The identity a front end stamps on its reports.
//!
//! A front end fills a [`Tool`] with its own values and passes it in
//! through [`RenderOptions`](super::RenderOptions). Each branding string must be
//! non-empty and hold no line break: the marker is written as one line, and
//! the others land in JSON fields and error messages read as single values.

use crate::policy::Policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub policy: Policy,
    /// The SARIF driver name, such as `cargo-crap`.
    pub name: &'static str,
    /// The version stamped on the JSON envelope and the SARIF driver. It
    /// must be the front end's own version, not that of the crate holding
    /// the renderers.
    pub version: &'static str,
    /// The SARIF driver's `informationUri` and its rule's `helpUri`.
    pub info_uri: &'static str,
    pub schemas: Schemas,
    /// The hidden first line of the markdown and PR-comment reports, which
    /// lets CI find and update its own comment. One HTML comment, such as
    /// `<!-- cargo-crap-report -->`.
    pub comment_marker: &'static str,
    /// The command named when a baseline does not parse, such as
    /// `cargo crap --format json`.
    pub baseline_hint: &'static str,
}

/// The `$schema` URLs of the two JSON envelopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schemas {
    pub report: &'static str,
    pub delta: &'static str,
}
