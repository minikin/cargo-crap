use super::config::SignalsConfig;
use crate::coverage::LineRange;
use serde::{Deserialize, Serialize};

/// The signals of one function. A recorded object missing a key reads it
/// as 0, so one partial entry cannot fail a whole baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Signals {
    /// Deepest number of nested bodies (`if`, `match`, loops, `let … else`,
    /// closures, `async` blocks) enclosing any code in the function.
    pub nesting: u32,
    /// The function's span in lines, signature to closing brace.
    pub lines: u32,
    /// Top-level statements of the body whose own nesting reaches 2.
    pub bumps: u32,
    /// Most `&&` / `||` operators in one `if` / `while` condition or guard.
    pub bool_ops: u32,
    /// `todo!`, `unimplemented!` and `dbg!` invocations.
    pub stubs: u32,
    /// `.unwrap()`, `.expect(…)` calls and `panic!` invocations.
    pub panics: u32,
}

/// Where each signal comes from: the line ranges a reason points at.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spans {
    /// The function's own span.
    pub lines: Vec<LineRange>,
    /// The top-level statement holding the deepest nesting (first of equals);
    /// empty when nesting is 0.
    pub nesting: Vec<LineRange>,
    pub bumps: Vec<LineRange>,
    /// The worst condition (first of equals); empty when `bool_ops` is 0.
    pub bool_ops: Vec<LineRange>,
    pub stubs: Vec<LineRange>,
    pub panics: Vec<LineRange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Measured {
    pub signals: Signals,
    pub spans: Spans,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    Nesting,
    Lines,
    Bumps,
    BoolOps,
    Stubs,
    Panics,
}

impl Signal {
    /// Every signal, in report order.
    pub const ALL: [Self; 6] = [
        Self::Nesting,
        Self::Lines,
        Self::Bumps,
        Self::BoolOps,
        Self::Stubs,
        Self::Panics,
    ];

    /// The default gate set: the signals whose regression fails
    /// `--fail-regression` unless `[signals] gate` says otherwise.
    pub const DEFAULT_GATE: [Self; 4] = [Self::Nesting, Self::Bumps, Self::BoolOps, Self::Stubs];

    /// The name as it appears in JSON and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nesting => "nesting",
            Self::Lines => "lines",
            Self::Bumps => "bumps",
            Self::BoolOps => "bool_ops",
            Self::Stubs => "stubs",
            Self::Panics => "panics",
        }
    }

    /// The signal a configured name means; `-` and `_` are interchangeable.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.replace('-', "_");
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    #[must_use]
    pub const fn value(
        self,
        signals: &Signals,
    ) -> u32 {
        match self {
            Self::Nesting => signals.nesting,
            Self::Lines => signals.lines,
            Self::Bumps => signals.bumps,
            Self::BoolOps => signals.bool_ops,
            Self::Stubs => signals.stubs,
            Self::Panics => signals.panics,
        }
    }

    #[must_use]
    pub fn spans(
        self,
        spans: &Spans,
    ) -> &[LineRange] {
        match self {
            Self::Nesting => &spans.nesting,
            Self::Lines => &spans.lines,
            Self::Bumps => &spans.bumps,
            Self::BoolOps => &spans.bool_ops,
            Self::Stubs => &spans.stubs,
            Self::Panics => &spans.panics,
        }
    }

    /// Counting signals compare a new function against zero; shape signals
    /// only against a configured limit.
    #[must_use]
    pub const fn is_count(self) -> bool {
        matches!(self, Self::Stubs | Self::Panics)
    }
}

/// Ceilings for the shape signals of new functions, validated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub nesting: Option<u32>,
    pub lines: Option<u32>,
    pub bumps: Option<u32>,
    pub bool_ops: Option<u32>,
}

impl Limits {
    /// The limit configured for `signal`; counting signals have none.
    #[must_use]
    pub const fn get(
        self,
        signal: Signal,
    ) -> Option<u32> {
        match signal {
            Signal::Nesting => self.nesting,
            Signal::Lines => self.lines,
            Signal::Bumps => self.bumps,
            Signal::BoolOps => self.bool_ops,
            Signal::Stubs | Signal::Panics => None,
        }
    }
}

/// The `[signals]` table, resolved and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalSettings {
    pub enabled: bool,
    /// Gated signals, deduplicated, in report order.
    pub gate: Vec<Signal>,
    pub limits: Limits,
}

impl Default for SignalSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            gate: Signal::DEFAULT_GATE.to_vec(),
            limits: Limits::default(),
        }
    }
}

impl SignalSettings {
    /// Resolve `config`, rejecting unknown gate names and negative limits.
    ///
    /// # Errors
    ///
    /// When a gate entry names no signal (the message names it and lists the
    /// valid ones) or a limit is negative or too large (the message names its
    /// key).
    pub fn resolve(config: &SignalsConfig) -> anyhow::Result<Self> {
        let gate = match &config.gate {
            None => Signal::DEFAULT_GATE.to_vec(),
            Some(names) => {
                let mut gate = Vec::with_capacity(names.len());
                for name in names {
                    let Some(signal) = Signal::parse(name) else {
                        anyhow::bail!(
                            "invalid signals.gate entry {name:?} (config): valid signals are \
                             nesting, lines, bumps, bool-ops, stubs, panics"
                        );
                    };
                    gate.push(signal);
                }
                gate.sort_unstable();
                gate.dedup();
                gate
            },
        };
        let raw = config.limits;
        let limits = Limits {
            nesting: limit("nesting", raw.nesting)?,
            lines: limit("lines", raw.lines)?,
            bumps: limit("bumps", raw.bumps)?,
            bool_ops: limit("bool-ops", raw.bool_ops)?,
        };
        Ok(Self {
            enabled: config.enabled.unwrap_or(false),
            gate,
            limits,
        })
    }
}

fn limit(
    key: &str,
    value: Option<i64>,
) -> anyhow::Result<Option<u32>> {
    value
        .map(|v| {
            u32::try_from(v).map_err(|_| {
                anyhow::anyhow!(
                    "invalid signals.limits.{key} value {v} (config): must be a non-negative \
                     integer no greater than {}",
                    u32::MAX
                )
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolving_signal_names_orders_and_deduplicates_aliases() {
        let config = SignalsConfig {
            enabled: Some(true),
            gate: Some(["bool-ops", "lines", "bool_ops"].map(str::to_owned).into()),
            ..Default::default()
        };
        let settings = SignalSettings::resolve(&config).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.gate, [Signal::Lines, Signal::BoolOps]);
        let unknown = SignalsConfig {
            gate: Some(vec!["unknown".into()]),
            ..config
        };
        let err = SignalSettings::resolve(&unknown).unwrap_err().to_string();
        assert!(
            err.contains("unknown") && err.contains("valid signals"),
            "{err}"
        );
    }

    #[test]
    fn resolved_limits_accept_the_u32_range_and_identify_out_of_range_values() {
        use crate::signals::config::SignalLimitsConfig;
        let settings = |value| {
            SignalSettings::resolve(&SignalsConfig {
                limits: SignalLimitsConfig {
                    nesting: Some(value),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        assert_eq!(settings(0).unwrap().limits.nesting, Some(0));
        assert_eq!(
            settings(i64::from(u32::MAX)).unwrap().limits.nesting,
            Some(u32::MAX)
        );
        for value in [-1, i64::from(u32::MAX) + 1] {
            let message = settings(value).unwrap_err().to_string();
            assert!(
                message.contains("signals.limits.nesting") && message.contains(&value.to_string()),
                "{message}"
            );
        }
    }

    #[test]
    fn a_partial_signals_object_reads_missing_keys_as_zero() {
        let s: Signals = serde_json::from_str(r#"{"nesting": 3}"#).expect("partial reads");
        assert_eq!(
            s,
            Signals {
                nesting: 3,
                ..Signals::default()
            }
        );
    }

    #[test]
    fn every_signal_parses_from_its_own_name_and_reads_its_own_field() {
        let signals = Signals {
            nesting: 1,
            lines: 2,
            bumps: 3,
            bool_ops: 4,
            stubs: 5,
            panics: 6,
        };
        for (i, signal) in Signal::ALL.into_iter().enumerate() {
            assert_eq!(Signal::parse(signal.name()), Some(signal));
            assert_eq!(signal.value(&signals), u32::try_from(i + 1).expect("small"));
        }
        assert_eq!(Signal::parse("bool-ops"), Some(Signal::BoolOps));
        assert_eq!(Signal::parse("crap"), None);
        assert!(Signal::Stubs.is_count() && Signal::Panics.is_count());
        assert!(!Signal::Nesting.is_count() && !Signal::Lines.is_count());
    }

    #[test]
    fn every_signal_reads_its_own_spans() {
        let range = |n| vec![LineRange { start: n, end: n }];
        let spans = Spans {
            lines: range(1),
            nesting: range(2),
            bumps: range(3),
            bool_ops: range(4),
            stubs: range(5),
            panics: range(6),
        };
        for (i, signal) in Signal::ALL.into_iter().enumerate() {
            let n = u32::try_from(i + 1).expect("small");
            let expected = if signal == Signal::Nesting {
                2
            } else if signal == Signal::Lines {
                1
            } else {
                n
            };
            assert_eq!(
                signal.spans(&spans),
                range(expected).as_slice(),
                "{signal:?}"
            );
        }
    }
}
