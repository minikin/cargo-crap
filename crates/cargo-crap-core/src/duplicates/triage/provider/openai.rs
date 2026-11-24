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
use crate::duplicates::triage::verdict::{
    DIVERGENCE_QUESTION, DecodeError, KIND_QUESTION, WORTH_QUESTION,
};
use crate::duplicates::verdict::WorthExtracting;
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
        /// Read only to find where the levels start, and never required: an
        /// unexpected shape is ignored, not rejected.
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
/// Decisions guide's numbering from 0 is assumed.
fn lowest_level(probabilities: &Value) -> f64 {
    let lowest = WorthExtracting::LABELS[0];
    probabilities
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry.get("label").and_then(Value::as_str) == Some(lowest))
        .and_then(|entry| entry.get("value"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}
