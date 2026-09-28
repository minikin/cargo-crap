//! What the model said about one pair, and how sure it has to be before the
//! report repeats it.

use serde::Deserialize;
use std::collections::HashMap;
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
    /// Decode a `/v1/systemone` response carrying the three answers.
    ///
    /// # Errors
    ///
    /// When the body is not a response, an answer is missing or of the wrong
    /// type, the kind is not one of the offered options, or a value is out
    /// of range.
    pub fn decode(body: &str) -> Result<Self, DecodeError> {
        let response: Response = serde_json::from_str(body)
            .map_err(|e| DecodeError(format!("not a /v1/systemone response: {e}")))?;
        let (choice, confidence) = response.choice(KIND_QUESTION)?;
        let kind = Kind::from_wire(&choice).ok_or_else(|| {
            DecodeError(format!(
                "{KIND_QUESTION}: {choice:?} is not an offered option"
            ))
        })?;
        Ok(Self {
            kind,
            confidence,
            worth_extracting: WorthExtracting(response.score(WORTH_QUESTION)?),
            divergence_risk: response.noul(DIVERGENCE_QUESTION)?,
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

impl std::error::Error for DecodeError {}

/// Values the server computes can land a hair past a bound. Within this of
/// one they are float noise, clamped to it rather than rejected: one
/// rejected pair discards the whole run's triage.
const TOLERANCE: f64 = 1e-9;

/// The parts of a `/v1/systemone` response this module reads. Answers stay
/// raw until asked for, so an answer nobody asked for is ignored and a
/// malformed one fails under its own question's name.
#[derive(Deserialize)]
struct Response {
    answers: HashMap<String, serde_json::Value>,
}

/// One answer; fields this module does not read (`probabilities`,
/// `legend`) are ignored.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Choice { choice: String, confidence: f64 },
    Score { score: f64, confidence: f64 },
    Noul { noul: f64 },
}

impl Answer {
    fn type_name(&self) -> &'static str {
        match self {
            Answer::Choice { .. } => "choice",
            Answer::Score { .. } => "score",
            Answer::Noul { .. } => "noul",
        }
    }
}

impl Response {
    fn answer(
        &self,
        id: &str,
    ) -> Result<Answer, DecodeError> {
        let raw = self
            .answers
            .get(id)
            .ok_or_else(|| DecodeError(format!("{id}: no answer")))?;
        Answer::deserialize(raw).map_err(|e| DecodeError(format!("{id}: {e}")))
    }

    /// The chosen option and the Choice's confidence.
    fn choice(
        &self,
        id: &str,
    ) -> Result<(String, f64), DecodeError> {
        match self.answer(id)? {
            Answer::Choice { choice, confidence } => {
                Ok((choice, within(id, "confidence", confidence, 1.0)?))
            },
            other => Err(wrong_type(id, "choice", &other)),
        }
    }

    /// The Score's position across the worth-extracting levels. Its
    /// confidence is checked but not kept: only the kind's confidence meets
    /// the floor.
    fn score(
        &self,
        id: &str,
    ) -> Result<f64, DecodeError> {
        match self.answer(id)? {
            Answer::Score { score, confidence } => {
                within(id, "confidence", confidence, 1.0)?;
                within(id, "score", score, WorthExtracting::TOP)
            },
            other => Err(wrong_type(id, "score", &other)),
        }
    }

    fn noul(
        &self,
        id: &str,
    ) -> Result<f64, DecodeError> {
        match self.answer(id)? {
            Answer::Noul { noul } => within(id, "noul", noul, 1.0),
            other => Err(wrong_type(id, "noul", &other)),
        }
    }
}

fn wrong_type(
    id: &str,
    expected: &str,
    got: &Answer,
) -> DecodeError {
    DecodeError(format!(
        "{id}: expected a {expected} answer, got {}",
        got.type_name()
    ))
}

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

    /// A full `/v1/systemone` response with the three answers, overridable
    /// one answer at a time.
    fn response(
        kind: &str,
        worth: &str,
        divergence: &str,
    ) -> String {
        format!(
            r#"{{"model":"jev-1.13.0","answers":{{"duplication_kind":{kind},"worth_extracting":{worth},"divergence_risk":{divergence}}},"usage":{{"input_tokens":900,"output_tokens":40}}}}"#
        )
    }

    const KIND: &str = r#"{"type":"choice","choice":"same_logic","probabilities":{"same_logic":0.86,"shared_shape_only":0.08,"structural_obligation":0.04,"parameterisable":0.02},"confidence":0.81}"#;
    const WORTH: &str = r#"{"type":"score","score":2.4,"legend":{"0":"a","1":"b","2":"c","3":"d"},"probabilities":{"0":0.0,"1":0.1,"2":0.4,"3":0.5},"confidence":0.62}"#;
    const DIVERGENCE: &str = r#"{"type":"noul","noul":0.8}"#;

    #[test]
    fn decodes_a_canned_response() {
        let verdict = Verdict::decode(&response(KIND, WORTH, DIVERGENCE)).expect("decodes");
        assert_eq!(verdict.kind, Kind::SameLogic);
        assert_eq!(verdict.confidence, 0.81);
        assert_eq!(verdict.worth_extracting.score(), 2.4);
        assert_eq!(verdict.worth_extracting.label(), "worthwhile");
        assert_eq!(verdict.divergence_risk, 0.8);
    }

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

    #[test]
    fn a_missing_answer_is_an_error_naming_the_question() {
        let body = r#"{"model":"jev","answers":{},"usage":{}}"#;
        let err = Verdict::decode(body).expect_err("nothing to decode");
        assert!(err.to_string().contains(KIND_QUESTION), "{err}");

        let without_worth = response(KIND, WORTH, DIVERGENCE)
            .replace(&format!(r#""worth_extracting":{WORTH},"#), "");
        let err = Verdict::decode(&without_worth).expect_err("no worth answer");
        assert!(err.to_string().contains(WORTH_QUESTION), "{err}");

        let without_divergence = response(KIND, WORTH, DIVERGENCE)
            .replace(&format!(r#","divergence_risk":{DIVERGENCE}"#), "");
        let err = Verdict::decode(&without_divergence).expect_err("no divergence answer");
        assert!(err.to_string().contains(DIVERGENCE_QUESTION), "{err}");
    }

    #[test]
    fn an_answer_of_the_wrong_type_is_an_error_naming_both_types() {
        // The message names the question, the type it needed and the type
        // that came back: the three facts a mismatched question set needs.
        let cases = [
            (
                response(DIVERGENCE, WORTH, DIVERGENCE),
                KIND_QUESTION,
                "a choice answer, got noul",
            ),
            (
                response(KIND, KIND, DIVERGENCE),
                WORTH_QUESTION,
                "a score answer, got choice",
            ),
            (
                response(KIND, WORTH, WORTH),
                DIVERGENCE_QUESTION,
                "a noul answer, got score",
            ),
        ];
        for (body, question, types) in cases {
            let err = Verdict::decode(&body)
                .expect_err("wrong answer type")
                .to_string();
            assert!(err.contains(question) && err.contains(types), "{err}");
        }
    }

    #[test]
    fn an_option_that_was_never_offered_is_an_error() {
        let kind = r#"{"type":"choice","choice":"copy_paste","probabilities":{},"confidence":0.9}"#;
        let err = Verdict::decode(&response(kind, WORTH, DIVERGENCE)).expect_err("unknown option");
        assert!(err.to_string().contains("copy_paste"), "{err}");
    }

    #[test]
    fn an_out_of_range_value_is_an_error() {
        let bad = [
            response(&KIND.replace("0.81", "1.2"), WORTH, DIVERGENCE),
            response(&KIND.replace("0.81", "-0.1"), WORTH, DIVERGENCE),
            response(KIND, &WORTH.replace("2.4", "3.5"), DIVERGENCE),
            response(KIND, &WORTH.replace("2.4", "-0.5"), DIVERGENCE),
            response(KIND, &WORTH.replace("0.62", "1.01"), DIVERGENCE),
            response(KIND, WORTH, &DIVERGENCE.replace("0.8", "1.5")),
            response(KIND, WORTH, &DIVERGENCE.replace("0.8", "-1")),
        ];
        for body in bad {
            assert!(Verdict::decode(&body).is_err(), "accepted: {body}");
        }
    }

    #[test]
    fn a_body_that_is_not_a_response_is_an_error() {
        assert!(Verdict::decode("not json").is_err());
        assert!(Verdict::decode(r#"{"error":"overloaded"}"#).is_err());
    }

    #[test]
    fn values_on_the_bounds_are_accepted() {
        // A certain Choice, a certain Noul either way, and both ends of the
        // Score are ordinary answers.
        for (confidence, noul) in [("1.0", "0.0"), ("0.0", "1.0")] {
            let body = response(
                &KIND.replace("0.81", confidence),
                &WORTH.replace("0.62", confidence),
                &DIVERGENCE.replace("0.8", noul),
            );
            let verdict = Verdict::decode(&body).expect("bounds are in range");
            assert_eq!(
                verdict.confidence.to_string(),
                confidence.trim_end_matches(".0")
            );
            assert_eq!(
                verdict.divergence_risk.to_string(),
                noul.trim_end_matches(".0")
            );
        }
    }

    #[test]
    fn float_noise_at_a_bound_is_clamped_to_it() {
        let body = response(
            &KIND.replace("0.81", "1.0000000002"),
            &WORTH.replace("2.4", "3.0000000001"),
            &DIVERGENCE.replace("0.8", "-0.0000000001"),
        );
        let verdict = Verdict::decode(&body).expect("noise within the tolerance");
        assert_eq!(verdict.confidence, 1.0);
        assert_eq!(verdict.worth_extracting.score(), 3.0);
        assert_eq!(verdict.divergence_risk, 0.0);

        // Past the tolerance it is still out of range.
        let body = response(&KIND.replace("0.81", "1.000001"), WORTH, DIVERGENCE);
        assert!(Verdict::decode(&body).is_err(), "1.000001 is not noise");
    }

    #[test]
    fn an_answer_nobody_asked_for_is_ignored() {
        let body = response(KIND, WORTH, DIVERGENCE).replace(
            r#""answers":{"#,
            r#""answers":{"diagnostic":{"type":"trace","id":"x"},"#,
        );
        let verdict = Verdict::decode(&body).expect("the three answers still decode");
        assert_eq!(verdict.kind, Kind::SameLogic);
    }

    #[test]
    fn a_malformed_answer_names_its_question() {
        let kind = r#"{"type":"choice","choice":"same_logic"}"#;
        let err = Verdict::decode(&response(kind, WORTH, DIVERGENCE))
            .expect_err("a choice without confidence")
            .to_string();
        assert!(
            err.contains(KIND_QUESTION) && err.contains("confidence"),
            "{err}"
        );

        let divergence = r#"{"type":"verdict","noul":0.8}"#;
        let err = Verdict::decode(&response(KIND, WORTH, divergence))
            .expect_err("an unknown answer type")
            .to_string();
        assert!(err.contains(DIVERGENCE_QUESTION), "{err}");
    }

    #[test]
    fn the_floor_is_inclusive() {
        let verdict = Verdict::decode(&response(KIND, WORTH, DIVERGENCE)).expect("decodes");
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
