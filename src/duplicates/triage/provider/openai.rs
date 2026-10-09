//! The `OpenAI` Decisions API: `POST /v1/decisions`.
//!
//! Its request carries the state as `input` text and the questions as an
//! array, each named by its id, with a choice's options in `choices` and a
//! score's levels in `levels`. The binary question is a predicate, which
//! carries no criteria, so its true and false rubrics are appended to its
//! instructions. The response carries the answers as an array, matched by
//! the echoed `name`. A question the model declines comes back as a
//! `refusal`, which is an error here: a refusal is not a judgment.

use crate::duplicates::triage::provider::{
    Answers, Provider, QuestionSet, offered_kind, wrong_type,
};
use crate::duplicates::triage::request::QUESTIONS;
use crate::duplicates::triage::verdict::{
    DIVERGENCE_QUESTION, DecodeError, KIND_QUESTION, WORTH_QUESTION,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The only place the API key is read from.
pub const API_KEY_VAR: &str = "OPENAI_API_KEY";
/// Overrides the API's base URL. It includes the `/v1` prefix, as it does
/// for the `OpenAI` SDKs.
pub const BASE_URL_VAR: &str = "OPENAI_BASE_URL";
/// The API's base URL, absent an override.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// The Decisions endpoint, joined to the base URL.
pub const ENDPOINT_PATH: &str = "/decisions";
/// The model asked when the configuration names none.
pub const DEFAULT_MODEL: &str = "gpt-6-luna";

/// The `OpenAI` provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAi;

impl Provider for OpenAi {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI API"
    }

    fn key_var(&self) -> &'static str {
        API_KEY_VAR
    }

    fn base_url_var(&self) -> &'static str {
        BASE_URL_VAR
    }

    fn default_base_url(&self) -> &'static str {
        DEFAULT_BASE_URL
    }

    fn endpoint_path(&self) -> &'static str {
        ENDPOINT_PATH
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    fn encode(
        &self,
        state: Value,
        model: &str,
        questions: &QuestionSet,
    ) -> Value {
        json!({
            "model": model,
            "input": state.to_string(),
            "questions": encode_questions(questions),
        })
    }

    fn decode(
        &self,
        body: &str,
    ) -> Result<Answers, DecodeError> {
        let response: Response = serde_json::from_str(body)
            .map_err(|e| DecodeError::new(format!("not a /v1/decisions response: {e}")))?;
        let (choice, kind_confidence) = response.choice(KIND_QUESTION)?;
        let kind = offered_kind(KIND_QUESTION, &choice)?;
        let (worth, worth_confidence) = response.score(WORTH_QUESTION)?;
        Ok(Answers {
            kind,
            kind_confidence,
            worth,
            worth_confidence,
            divergence: response.predicate(DIVERGENCE_QUESTION)?,
        })
    }
}

/// The questions as an array, in the set's order: the choice, the score,
/// then the predicate.
fn encode_questions(set: &QuestionSet) -> Value {
    let choices: Vec<Value> = set
        .kind
        .options
        .iter()
        .map(|(kind, rubric)| json!({"value": kind.wire_name(), "description": rubric}))
        .collect();
    let levels: Vec<Value> = set
        .worth
        .levels
        .iter()
        .map(|(label, rubric)| json!({"label": label, "description": rubric}))
        .collect();
    let predicate = format!(
        "{}\n\nTrue: {}\nFalse: {}",
        set.divergence.instructions, set.divergence.if_true, set.divergence.if_false
    );
    json!([
        {
            "type": "choice",
            "name": set.kind.id,
            "instructions": set.kind.instructions,
            "choices": choices,
        },
        {
            "type": "score",
            "name": set.worth.id,
            "instructions": set.worth.instructions,
            "levels": levels,
        },
        {
            "type": "predicate",
            "name": set.divergence.id,
            "instructions": predicate,
        },
    ])
}

/// The parts of a `/v1/decisions` response this module reads. Answers stay
/// raw until asked for, so an answer nobody asked for is ignored and a
/// malformed one fails under its own question's name.
#[derive(Deserialize)]
struct Response {
    answers: Vec<Value>,
}

/// One answer that is a judgment. Fields this module does not read are
/// ignored, and a refusal is caught before an answer is read as one of
/// these.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Answer {
    Choice {
        choice: String,
        confidence: f64,
    },
    Score {
        score: f64,
        confidence: f64,
        /// Read only to find where the levels start, and never required:
        /// a shape this module does not expect costs nothing.
        #[serde(default)]
        probabilities: Value,
    },
    Predicate {
        probability: f64,
    },
}

impl Answer {
    fn type_name(&self) -> &'static str {
        match self {
            Answer::Choice { .. } => "choice",
            Answer::Score { .. } => "score",
            Answer::Predicate { .. } => "predicate",
        }
    }
}

impl Response {
    /// The answer named `name`. A refusal is an error here, so no caller
    /// can mistake it for a judgment.
    fn answer(
        &self,
        name: &str,
    ) -> Result<Answer, DecodeError> {
        let mut named = self
            .answers
            .iter()
            .filter(|answer| answer.get("name").and_then(Value::as_str) == Some(name));
        let raw = named
            .next()
            .ok_or_else(|| DecodeError::new(format!("{name}: no answer")))?;
        if named.next().is_some() {
            return Err(DecodeError::new(format!("{name}: answered more than once")));
        }
        if raw.get("type").and_then(Value::as_str) == Some("refusal") {
            return Err(DecodeError::new(format!(
                "{name}: the question was refused"
            )));
        }
        Answer::deserialize(raw).map_err(|e| DecodeError::new(format!("{name}: {e}")))
    }

    /// The chosen option and the choice's confidence.
    fn choice(
        &self,
        name: &str,
    ) -> Result<(String, f64), DecodeError> {
        match self.answer(name)? {
            Answer::Choice { choice, confidence } => Ok((choice, confidence)),
            other => Err(wrong_type(name, "choice", other.type_name())),
        }
    }

    /// The score's position across the levels, counted from the lowest
    /// level sent, and its confidence.
    fn score(
        &self,
        name: &str,
    ) -> Result<(f64, f64), DecodeError> {
        match self.answer(name)? {
            Answer::Score {
                score,
                confidence,
                probabilities,
            } => Ok((score - lowest_level(&probabilities), confidence)),
            other => Err(wrong_type(name, "score", other.type_name())),
        }
    }

    fn predicate(
        &self,
        name: &str,
    ) -> Result<f64, DecodeError> {
        match self.answer(name)? {
            Answer::Predicate { probability } => Ok(probability),
            other => Err(wrong_type(name, "predicate", other.type_name())),
        }
    }
}

/// The index the response gives the lowest level sent, found by the label
/// it echoes back. When no entry carries that label and a number, the
/// guide's numbering from 0 is assumed.
fn lowest_level(probabilities: &Value) -> f64 {
    let lowest = QUESTIONS.worth.levels[0].0;
    probabilities
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry.get("label").and_then(Value::as_str) == Some(lowest))
        .and_then(|entry| entry.get("value"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::triage::provider::typesafe::TypeSafe;
    use crate::duplicates::triage::request::QUESTIONS;
    use crate::duplicates::triage::verdict::{
        DIVERGENCE_QUESTION, KIND_QUESTION, Kind, Verdict, WORTH_QUESTION, WorthExtracting,
    };
    use proptest::prelude::*;
    use serde_json::json;

    /// A `/v1/decisions` response carrying `answers` under the names the
    /// encoded questions use, in the shapes the guide documents.
    fn response_for(answers: &Answers) -> String {
        let body = OpenAi.encode(json!({}), "gpt-6-luna", &QUESTIONS);
        let mut out = Vec::new();
        for question in body["questions"].as_array().expect("an array") {
            let name = question["name"].clone();
            out.push(match question["type"].as_str().expect("a type") {
                "choice" => json!({
                    "type": "choice",
                    "name": name,
                    "choice": answers.kind.wire_name(),
                    "probabilities": [],
                    "confidence": answers.kind_confidence,
                }),
                "score" => json!({
                    "type": "score",
                    "name": name,
                    "score": answers.worth,
                    "probabilities": [
                        {"value": 0, "label": "leave-it", "probability": 0.25},
                        {"value": 3, "label": "should-be-one", "probability": 0.25}
                    ],
                    "confidence": answers.worth_confidence,
                }),
                other => {
                    assert_eq!(other, "predicate");
                    json!({"type": "predicate", "name": name, "probability": answers.divergence})
                },
            });
        }
        json!({ "answers": out }).to_string()
    }

    const KIND: &str = r#"{"type":"choice","name":"duplication_kind","choice":"same_logic","probabilities":[{"value":"same_logic","probability":0.86},{"value":"shared_shape_only","probability":0.14}],"confidence":0.81}"#;
    const WORTH: &str = r#"{"type":"score","name":"worth_extracting","score":2.4,"probabilities":[{"value":0,"label":"leave-it","probability":0.0},{"value":1,"label":"optional","probability":0.1},{"value":2,"label":"worthwhile","probability":0.4},{"value":3,"label":"should-be-one","probability":0.5}],"confidence":0.62}"#;
    const DIVERGENCE: &str = r#"{"type":"predicate","name":"divergence_risk","probability":0.8}"#;

    fn response(answers: &[&str]) -> String {
        format!(r#"{{"answers":[{}]}}"#, answers.join(","))
    }

    #[test]
    fn openai_names_its_api_and_environment() {
        let provider = OpenAi;
        assert_eq!(provider.id(), "openai");
        assert_eq!(provider.display_name(), "OpenAI API");
        assert_eq!(provider.key_var(), "OPENAI_API_KEY");
        assert_eq!(provider.base_url_var(), "OPENAI_BASE_URL");
        assert_eq!(provider.default_base_url(), "https://api.openai.com/v1");
        assert_eq!(provider.endpoint_path(), "/decisions");
        assert_eq!(provider.default_model(), "gpt-6-luna");
    }

    #[test]
    fn the_body_names_the_model_and_carries_the_state_as_input_text() {
        let state = json!({"function_a": {"name": "alpha", "source": "fn alpha() {}"}});
        let body = OpenAi.encode(state.clone(), "gpt-6-luna", &QUESTIONS);
        assert_eq!(body["model"], "gpt-6-luna");
        let input = body["input"].as_str().expect("input is text");
        assert_eq!(
            serde_json::from_str::<Value>(input).expect("JSON text"),
            state
        );
        assert_eq!(body.as_object().expect("a map").len(), 3, "{body}");
    }

    #[test]
    fn the_three_questions_are_a_choice_a_score_and_a_predicate() {
        let body = OpenAi.encode(json!({}), "gpt-6-luna", &QUESTIONS);
        let questions = body["questions"].as_array().expect("an array");
        let shape: Vec<(&str, &str)> = questions
            .iter()
            .map(|q| (q["type"].as_str().unwrap(), q["name"].as_str().unwrap()))
            .collect();
        assert_eq!(
            shape,
            [
                ("choice", KIND_QUESTION),
                ("score", WORTH_QUESTION),
                ("predicate", DIVERGENCE_QUESTION),
            ]
        );
        assert_eq!(questions[0]["instructions"], QUESTIONS.kind.instructions);
        assert_eq!(questions[1]["instructions"], QUESTIONS.worth.instructions);
    }

    #[test]
    fn every_kind_is_offered_as_a_choice_with_its_rubric() {
        let body = OpenAi.encode(json!({}), "gpt-6-luna", &QUESTIONS);
        let choices = body["questions"][0]["choices"].as_array().expect("choices");
        let sent: Vec<(&str, &str)> = choices
            .iter()
            .map(|c| {
                (
                    c["value"].as_str().unwrap(),
                    c["description"].as_str().unwrap(),
                )
            })
            .collect();
        let expected: Vec<(&str, &str)> = QUESTIONS
            .kind
            .options
            .iter()
            .map(|(kind, rubric)| (kind.wire_name(), *rubric))
            .collect();
        assert_eq!(sent, expected);
        assert_eq!(sent.len(), Kind::ALL.len());
    }

    #[test]
    fn worth_levels_are_sent_lowest_first_with_their_labels() {
        let body = OpenAi.encode(json!({}), "gpt-6-luna", &QUESTIONS);
        let levels = body["questions"][1]["levels"].as_array().expect("levels");
        let sent: Vec<(&str, &str)> = levels
            .iter()
            .map(|l| {
                (
                    l["label"].as_str().unwrap(),
                    l["description"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(sent, QUESTIONS.worth.levels);
        let labels: Vec<&str> = sent.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, WorthExtracting::LABELS);
    }

    #[test]
    fn the_predicate_carries_both_rubrics_in_its_instructions() {
        let body = OpenAi.encode(json!({}), "gpt-6-luna", &QUESTIONS);
        let predicate = &body["questions"][2];
        let instructions = predicate["instructions"].as_str().expect("text");
        assert!(
            instructions.starts_with(QUESTIONS.divergence.instructions),
            "{instructions}"
        );
        assert!(
            instructions.contains(QUESTIONS.divergence.if_true),
            "{instructions}"
        );
        assert!(
            instructions.contains(QUESTIONS.divergence.if_false),
            "{instructions}"
        );
        assert_eq!(
            predicate.as_object().expect("a map").len(),
            3,
            "type, name, instructions"
        );
    }

    #[test]
    fn decodes_the_three_documented_answer_shapes() {
        let answers = OpenAi
            .decode(&response(&[KIND, WORTH, DIVERGENCE]))
            .expect("decodes");
        assert_eq!(
            answers,
            Answers {
                kind: Kind::SameLogic,
                kind_confidence: 0.81,
                worth: 2.4,
                worth_confidence: 0.62,
                divergence: 0.8,
            }
        );
    }

    #[test]
    fn answers_are_matched_by_name_not_position() {
        let answers = OpenAi
            .decode(&response(&[DIVERGENCE, WORTH, KIND]))
            .expect("decodes in any order");
        assert_eq!(answers.kind, Kind::SameLogic);
        assert_eq!(answers.divergence, 0.8);
    }

    #[test]
    fn a_refused_question_is_an_error_that_says_so() {
        let refused = r#"{"type":"refusal","name":"worth_extracting"}"#;
        let err = OpenAi
            .decode(&response(&[KIND, refused, DIVERGENCE]))
            .expect_err("a refusal is not a judgment")
            .to_string();
        assert_eq!(err, "worth_extracting: the question was refused");
    }

    #[test]
    fn a_choice_that_names_no_kind_is_an_error_naming_the_question_and_value() {
        let kind =
            r#"{"type":"choice","name":"duplication_kind","choice":"copy_paste","confidence":0.9}"#;
        let err = OpenAi
            .decode(&response(&[kind, WORTH, DIVERGENCE]))
            .expect_err("unknown option")
            .to_string();
        assert_eq!(
            err,
            r#"duplication_kind: "copy_paste" is not an offered option"#
        );
    }

    #[test]
    fn a_missing_answer_is_an_error_naming_the_question() {
        for (present, missing) in [
            ([WORTH, DIVERGENCE], KIND_QUESTION),
            ([KIND, DIVERGENCE], WORTH_QUESTION),
            ([KIND, WORTH], DIVERGENCE_QUESTION),
        ] {
            let err = OpenAi
                .decode(&response(&present))
                .expect_err("an answer is missing")
                .to_string();
            assert_eq!(err, format!("{missing}: no answer"));
        }
    }

    #[test]
    fn an_answer_of_the_wrong_type_is_an_error_naming_both_types() {
        let as_score = WORTH.replace("worth_extracting", "duplication_kind");
        let err = OpenAi
            .decode(&response(&[&as_score, WORTH, DIVERGENCE]))
            .expect_err("a score where a choice was asked")
            .to_string();
        assert_eq!(err, "duplication_kind: expected a choice answer, got score");
        let as_choice = KIND.replace("duplication_kind", "divergence_risk");
        let err = OpenAi
            .decode(&response(&[KIND, WORTH, &as_choice]))
            .expect_err("a choice where a predicate was asked")
            .to_string();
        assert_eq!(
            err,
            "divergence_risk: expected a predicate answer, got choice"
        );
        let as_predicate = DIVERGENCE.replace("divergence_risk", "worth_extracting");
        let err = OpenAi
            .decode(&response(&[KIND, &as_predicate, DIVERGENCE]))
            .expect_err("a predicate where a score was asked")
            .to_string();
        assert_eq!(
            err,
            "worth_extracting: expected a score answer, got predicate"
        );
    }

    #[test]
    fn a_malformed_answer_names_its_question() {
        let kind = r#"{"type":"choice","name":"duplication_kind","choice":"same_logic"}"#;
        let err = OpenAi
            .decode(&response(&[kind, WORTH, DIVERGENCE]))
            .expect_err("a choice without confidence")
            .to_string();
        assert!(
            err.starts_with("duplication_kind: ") && err.contains("confidence"),
            "{err}"
        );
    }

    #[test]
    fn a_body_that_is_not_a_response_is_an_error() {
        let err = OpenAi.decode("not json").expect_err("not JSON").to_string();
        assert!(err.starts_with("not a /v1/decisions response: "), "{err}");
        let err = OpenAi
            .decode(r#"{"error":{"message":"overloaded"}}"#)
            .expect_err("no answers")
            .to_string();
        assert!(err.starts_with("not a /v1/decisions response: "), "{err}");
    }

    #[test]
    fn an_answer_nobody_asked_for_is_ignored() {
        let extra = r#"{"type":"predicate","name":"diagnostic","probability":0.1}"#;
        let answers = OpenAi
            .decode(&response(&[extra, KIND, WORTH, DIVERGENCE]))
            .expect("the three answers still decode");
        assert_eq!(answers.kind, Kind::SameLogic);
    }

    /// A worth answer of `score` whose probabilities are `levels`.
    fn worth(
        score: f64,
        levels: &str,
    ) -> String {
        format!(
            r#"{{"type":"score","name":"worth_extracting","score":{score},"probabilities":{levels},"confidence":0.62}}"#
        )
    }

    fn decoded_worth(answer: &str) -> f64 {
        OpenAi
            .decode(&response(&[KIND, answer, DIVERGENCE]))
            .expect("decodes")
            .worth
    }

    #[test]
    fn a_score_is_counted_from_the_lowest_level_sent() {
        // Numbered from 1: the level labelled leave-it reads 0.
        let one_based = worth(
            3.4,
            r#"[{"value":1,"label":"leave-it","probability":0.0},{"value":2,"label":"optional","probability":0.1},{"value":3,"label":"worthwhile","probability":0.4},{"value":4,"label":"should-be-one","probability":0.5}]"#,
        );
        assert!((decoded_worth(&one_based) - 2.4).abs() < 1e-12);
        // The bottom level left out: nothing to anchor on, so the guide's
        // numbering from 0 holds and the score is not shifted.
        let bottom_dropped = worth(
            2.4,
            r#"[{"value":1,"label":"optional","probability":0.1},{"value":2,"label":"worthwhile","probability":0.4},{"value":3,"label":"should-be-one","probability":0.5}]"#,
        );
        assert_eq!(decoded_worth(&bottom_dropped), 2.4);
        // No probabilities at all.
        let unreported =
            r#"{"type":"score","name":"worth_extracting","score":2.4,"confidence":0.62}"#;
        assert_eq!(decoded_worth(unreported), 2.4);
    }

    #[test]
    fn probabilities_in_an_unexpected_shape_never_fail_the_answer() {
        let shapes = [
            r#"[{"label":"leave-it","probability":0.1}]"#,
            r#"[{"value":"0","label":"leave-it","probability":0.1}]"#,
            r#"{"leave-it":0.1}"#,
            r"[0.1, 0.2]",
        ];
        for levels in shapes {
            assert_eq!(decoded_worth(&worth(2.4, levels)), 2.4, "{levels}");
        }
    }

    #[test]
    fn a_question_answered_twice_is_an_error() {
        let refused = r#"{"type":"refusal","name":"duplication_kind"}"#;
        let err = OpenAi
            .decode(&response(&[KIND, refused, WORTH, DIVERGENCE]))
            .expect_err("two answers to one question")
            .to_string();
        assert_eq!(err, "duplication_kind: answered more than once");
    }

    fn answers() -> impl Strategy<Value = Answers> {
        (
            0..Kind::ALL.len(),
            0.0..=1.0f64,
            0.0..=3.0f64,
            0.0..=1.0f64,
            0.0..=1.0f64,
        )
            .prop_map(
                |(kind, kind_confidence, worth, worth_confidence, divergence)| Answers {
                    kind: Kind::ALL[kind],
                    kind_confidence,
                    worth,
                    worth_confidence,
                    divergence,
                },
            )
    }

    /// The same answers as `TypeSafe` writes them.
    fn typesafe_response(answers: &Answers) -> String {
        json!({
            "model": "jev",
            "answers": {
                KIND_QUESTION: {
                    "type": "choice",
                    "choice": answers.kind.wire_name(),
                    "confidence": answers.kind_confidence,
                },
                WORTH_QUESTION: {
                    "type": "score",
                    "score": answers.worth,
                    "confidence": answers.worth_confidence,
                },
                DIVERGENCE_QUESTION: {"type": "noul", "noul": answers.divergence},
            },
        })
        .to_string()
    }

    proptest! {
        /// Every question encoded is answered by the name decode reads, and
        /// every answer it reads comes back as sent.
        #[test]
        fn encode_and_decode_agree(answers in answers()) {
            let decoded = OpenAi.decode(&response_for(&answers)).expect("decodes");
            prop_assert_eq!(decoded, answers);
        }

        /// The same judgment, in either provider's wire shape, is the same
        /// verdict.
        #[test]
        fn either_wire_shape_decodes_to_the_same_verdict(answers in answers()) {
            let openai = OpenAi
                .decode(&response_for(&answers))
                .and_then(Verdict::from_answers)
                .expect("decodes");
            let typesafe = TypeSafe
                .decode(&typesafe_response(&answers))
                .and_then(Verdict::from_answers)
                .expect("decodes");
            prop_assert_eq!(openai, typesafe);
        }
    }
}
