//! Configuration of structural signals and their limits.

use serde::Deserialize;

/// The `[signals]` table.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SignalsConfig {
    /// Compute and report the signals. Defaults to `false`.
    pub enabled: Option<bool>,

    /// Signals whose regression trips `--fail-regression`, by name (`-` or
    /// `_` accepted). Defaults to `nesting`, `bumps`, `bool-ops`, `stubs`.
    /// Validated against the known names after loading.
    pub gate: Option<Vec<String>>,

    /// Ceilings for the shape signals of functions new since the baseline.
    #[serde(default)]
    pub limits: SignalLimitsConfig,
}

/// The `[signals.limits]` table, as written. Signed so a negative value
/// reaches validation and is reported by its key.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SignalLimitsConfig {
    pub nesting: Option<i64>,
    pub lines: Option<i64>,
    pub bumps: Option<i64>,
    /// Accepted as `bool-ops` (house style) or `bool_ops`.
    #[serde(alias = "bool_ops")]
    pub bool_ops: Option<i64>,
}
