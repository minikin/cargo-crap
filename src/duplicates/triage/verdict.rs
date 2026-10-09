//! What the model said about one pair, and how sure it has to be before the
//! report repeats it.

use crate::duplicates::triage::provider::Answers;
use std::fmt;

/// Question id of the Choice that names the kind of duplication.
pub const KIND_QUESTION: &str = "duplication_kind";
/// Question id of the Score that rates whether the pair is worth extracting.
pub const WORTH_QUESTION: &str = "worth_extracting";
/// Question id of the Noul that asks whether a fix to one side would likely
/// miss the other.
pub const DIVERGENCE_QUESTION: &str = "divergence_risk";

/// What kind of duplication a pair is: the question the similarity score
/// cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The same logic, written twice.
    SameLogic,
    /// Nothing in common but a Rust idiom or a sequence of similar calls.
    SharedShapeOnly,
    /// A shape imposed from outside: a trait, a macro, a visitor.
    StructuralObligation,
    /// The same logic up to a value or a type that could be a parameter.
    Parameterisable,
}

impl Kind {
    /// Every kind. A Choice's answer is an option key, not a position, so
    /// this order carries no meaning on the wire.
    pub const ALL: [Kind; 4] = [
        Kind::SameLogic,
        Kind::SharedShapeOnly,
        Kind::StructuralObligation,
        Kind::Parameterisable,
    ];

    /// The option key on the wire.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Kind::SameLogic => "same_logic",
            Kind::SharedShapeOnly => "shared_shape_only",
            Kind::StructuralObligation => "structural_obligation",
            Kind::Parameterisable => "parameterisable",
        }
    }

    /// The label the report prints.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Kind::SameLogic => "same-logic",
            Kind::SharedShapeOnly => "shared-shape-only",
            Kind::StructuralObligation => "structural-obligation",
            Kind::Parameterisable => "parameterisable",
        }
    }

    /// The kind whose wire name is `name`, if any.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.wire_name() == name)
    }
}

/// Whether the pair should become one function: the Score's
/// probability-weighted position across four ordered levels, `0.0..=3.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorthExtracting(f64);

impl WorthExtracting {
    /// The levels' labels, from "leave it" to "should be one function".
    pub const LABELS: [&'static str; 4] = ["leave-it", "optional", "worthwhile", "should-be-one"];

    /// The top level's score; the bottom is `0.0`.
    pub const TOP: f64 = (Self::LABELS.len() - 1) as f64;

    /// A score within the levels, or `None` outside them (or NaN).
    #[must_use]
    pub fn new(score: f64) -> Option<Self> {
        (0.0..=Self::TOP).contains(&score).then_some(Self(score))
    }

    /// The score as the model returned it.
    #[must_use]
    pub fn score(self) -> f64 {
        self.0
    }

    /// The label of the level nearest the score.
    #[must_use]
    pub fn label(self) -> &'static str {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the score is range-checked to 0.0..=3.0, so the rounded level is 0..=3"
        )]
        let level = self.0.round() as usize;
        Self::LABELS[level]
    }
}

/// The model's three answers about one pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub kind: Kind,
    /// The kind Choice's confidence, `0.0..=1.0`. What the floor compares.
    pub confidence: f64,
    pub worth_extracting: WorthExtracting,
    /// Probability that a fix to one side would be missed in the other.
    pub divergence_risk: f64,
}

/// A verdict as the report may repeat it: its kind when the model was sure
/// enough, only its confidence when it was not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Assessment {
    /// Confidence at or above the floor: the verdict stands.
    Kind(Verdict),
    /// Confidence below the floor: no kind is asserted.
    Uncertain { confidence: f64 },
}

impl Verdict {
    /// The verdict `answers` carry, every value checked against its range.
    /// Every provider's answers pass through here, so every provider gets
    /// the same checks and the same clamp.
    ///
    /// # Errors
    ///
    /// When a confidence or the divergence probability is outside `0..=1`,
    /// or the worth score is outside its levels, NaN included. The error
    /// names the question and the field.
    pub fn from_answers(answers: Answers) -> Result<Self, DecodeError> {
        let confidence = within(KIND_QUESTION, "confidence", answers.kind_confidence, 1.0)?;
        // Checked, not kept: only the kind's confidence meets the floor.
        within(WORTH_QUESTION, "confidence", answers.worth_confidence, 1.0)?;
        let worth = within(WORTH_QUESTION, "score", answers.worth, WorthExtracting::TOP)?;
        Ok(Self {
            kind: answers.kind,
            confidence,
            worth_extracting: WorthExtracting(worth),
            divergence_risk: within(DIVERGENCE_QUESTION, "probability", answers.divergence, 1.0)?,
        })
    }

    /// Assert the kind when the confidence reaches `floor`; otherwise report
    /// only that the model was unsure. The model saying "I don't know" is a
    /// result, not a failure.
    #[must_use]
    pub fn assessment(
        self,
        floor: f64,
    ) -> Assessment {
        if self.confidence >= floor {
            Assessment::Kind(self)
        } else {
            Assessment::Uncertain {
                confidence: self.confidence,
            }
        }
    }
}

/// Why a response could not be decoded into a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(String);

impl fmt::Display for DecodeError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl DecodeError {
    /// An error whose message is `message`.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::error::Error for DecodeError {}

/// Values the server computes can land a hair past a bound. Within this of
/// one they are float noise, clamped to it rather than rejected: one
/// rejected pair discards the whole run's triage.
const TOLERANCE: f64 = 1e-9;

/// `value` clamped into `0.0..=top` when it lies there or within
/// [`TOLERANCE`] of an end; an error naming the field otherwise, NaN
/// included.
fn within(
    id: &str,
    field: &str,
    value: f64,
    top: f64,
) -> Result<f64, DecodeError> {
    if (-TOLERANCE..=top + TOLERANCE).contains(&value) {
        Ok(value.clamp(0.0, top))
    } else {
        Err(DecodeError(format!(
            "{id}: {field} {value} is outside 0..={top}"
        )))
    }
}

#[cfg(test)]
#[expect(
    clippy::float_cmp,
    reason = "decoded values are compared with the exact literals the canned responses contain"
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn every_kind_has_one_wire_name_and_one_kebab_label() {
        let wire = [
            "same_logic",
            "shared_shape_only",
            "structural_obligation",
            "parameterisable",
        ];
        let labels = [
            "same-logic",
            "shared-shape-only",
            "structural-obligation",
            "parameterisable",
        ];
        for ((kind, wire), label) in Kind::ALL.into_iter().zip(wire).zip(labels) {
            assert_eq!(kind.wire_name(), wire);
            assert_eq!(kind.label(), label);
            assert_eq!(Kind::from_wire(wire), Some(kind));
        }
        assert_eq!(
            Kind::from_wire("same-logic"),
            None,
            "labels are not wire names"
        );
    }

    fn answers() -> Answers {
        Answers {
            kind: Kind::SameLogic,
            kind_confidence: 0.81,
            worth: 2.4,
            worth_confidence: 0.62,
            divergence: 0.8,
        }
    }

    #[test]
    fn answers_in_range_become_the_verdict() {
        let verdict = Verdict::from_answers(answers()).expect("in range");
        assert_eq!(verdict.kind, Kind::SameLogic);
        assert_eq!(verdict.confidence, 0.81);
        assert_eq!(verdict.worth_extracting.score(), 2.4);
        assert_eq!(verdict.divergence_risk, 0.8);
    }

    #[test]
    fn an_out_of_range_answer_is_an_error_naming_its_question_and_field() {
        type Spoil = fn(&mut Answers);
        let cases: [(Spoil, &str); 9] = [
            (
                |a| a.kind_confidence = 1.2,
                "duplication_kind: confidence 1.2",
            ),
            (
                |a| a.kind_confidence = -0.1,
                "duplication_kind: confidence -0.1",
            ),
            (|a| a.worth = 3.5, "worth_extracting: score 3.5"),
            (|a| a.worth = -0.5, "worth_extracting: score -0.5"),
            (
                |a| a.worth_confidence = 1.01,
                "worth_extracting: confidence 1.01",
            ),
            (
                |a| a.worth_confidence = -0.2,
                "worth_extracting: confidence -0.2",
            ),
            (|a| a.divergence = 1.5, "divergence_risk: probability 1.5"),
            (|a| a.divergence = -1.0, "divergence_risk: probability -1"),
            (
                |a| a.divergence = f64::NAN,
                "divergence_risk: probability NaN",
            ),
        ];
        for (spoil, message) in cases {
            let mut bad = answers();
            spoil(&mut bad);
            let err = Verdict::from_answers(bad)
                .expect_err("out of range")
                .to_string();
            assert!(
                err.starts_with(message) && err.contains("is outside 0..="),
                "{err}"
            );
        }
    }

    #[test]
    fn a_score_and_its_confidence_both_out_of_range_name_the_confidence() {
        // The order the checks ran in before triage had providers.
        let bad = Answers {
            worth: 9.0,
            worth_confidence: 9.0,
            ..answers()
        };
        let err = Verdict::from_answers(bad)
            .expect_err("both out of range")
            .to_string();
        assert!(err.starts_with("worth_extracting: confidence 9"), "{err}");
    }

    #[test]
    fn answers_on_the_bounds_and_within_noise_of_them_are_clamped() {
        let low = Answers {
            kind_confidence: -1e-12,
            worth: 0.0,
            worth_confidence: 0.0,
            divergence: -1e-12,
            ..answers()
        };
        let verdict = Verdict::from_answers(low).expect("noise at the bottom");
        assert_eq!(verdict.confidence, 0.0);
        assert_eq!(verdict.worth_extracting.score(), 0.0);
        assert_eq!(verdict.divergence_risk, 0.0);
        let high = Answers {
            kind_confidence: 1.0 + 1e-12,
            worth: 3.0 + 1e-12,
            worth_confidence: 1.0,
            divergence: 1.0,
            ..answers()
        };
        let verdict = Verdict::from_answers(high).expect("noise at the top");
        assert_eq!(verdict.confidence, 1.0);
        assert_eq!(verdict.worth_extracting.score(), 3.0);
        assert_eq!(verdict.divergence_risk, 1.0);
        let past = Answers {
            kind_confidence: 1.000_001,
            ..answers()
        };
        assert!(
            Verdict::from_answers(past).is_err(),
            "1.000001 is not noise"
        );
    }

    #[test]
    fn the_floor_is_inclusive() {
        let verdict = Verdict::from_answers(answers()).expect("in range");
        assert_eq!(verdict.assessment(0.81), Assessment::Kind(verdict));
        assert_eq!(
            verdict.assessment(0.82),
            Assessment::Uncertain { confidence: 0.81 }
        );
    }

    #[test]
    fn the_worth_label_is_the_nearest_level() {
        let cases = [
            (0.0, "leave-it"),
            (0.49, "leave-it"),
            (0.5, "optional"),
            (1.2, "optional"),
            (1.6, "worthwhile"),
            (2.49, "worthwhile"),
            (2.6, "should-be-one"),
            (3.0, "should-be-one"),
        ];
        for (score, label) in cases {
            let worth = WorthExtracting::new(score).expect("in range");
            assert_eq!(worth.label(), label, "score {score}");
        }
        assert_eq!(WorthExtracting::new(3.01), None);
        assert_eq!(WorthExtracting::new(-0.01), None);
        assert_eq!(WorthExtracting::new(f64::NAN), None);
    }

    fn verdicts() -> impl Strategy<Value = Verdict> {
        (0..Kind::ALL.len(), 0.0..=1.0f64, 0.0..=3.0f64, 0.0..=1.0f64).prop_map(
            |(kind, confidence, worth, divergence_risk)| Verdict {
                kind: Kind::ALL[kind],
                confidence,
                worth_extracting: WorthExtracting::new(worth).expect("in range"),
                divergence_risk,
            },
        )
    }

    proptest! {
        /// Every verdict either asserts its kind or is uncertain, decided by
        /// the confidence against the floor and nothing else.
        #[test]
        fn the_floor_is_a_partition(verdict in verdicts(), floor in 0.0..=1.0f64) {
            match verdict.assessment(floor) {
                Assessment::Kind(asserted) => {
                    prop_assert!(verdict.confidence >= floor);
                    prop_assert_eq!(asserted, verdict, "asserted unchanged");
                }
                Assessment::Uncertain { confidence } => {
                    prop_assert!(verdict.confidence < floor);
                    prop_assert_eq!(confidence, verdict.confidence);
                }
            }
        }

        /// A stricter floor can only take assertions away.
        #[test]
        fn raising_the_floor_never_asserts_a_kind(
            verdict in verdicts(),
            a in 0.0..=1.0f64,
            b in 0.0..=1.0f64,
        ) {
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            if matches!(verdict.assessment(low), Assessment::Uncertain { .. }) {
                prop_assert!(
                    matches!(verdict.assessment(high), Assessment::Uncertain { .. }),
                    "uncertain at {} but asserted at {}", low, high
                );
            }
        }

        /// The label is the level nearest the score, never more than half a
        /// level away.
        #[test]
        fn the_worth_label_is_within_half_a_level(score in 0.0..=3.0f64) {
            let worth = WorthExtracting::new(score).expect("in range");
            let level = WorthExtracting::LABELS
                .iter()
                .position(|l| *l == worth.label())
                .expect("a known label");
            prop_assert!((score - level as f64).abs() <= 0.5, "{} labelled {}", score, worth.label());
        }
    }
}
