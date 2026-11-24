//! The `TypeSafe` System One API: `POST /v1/systemone`.
//!
//! Its request carries the state as an object and the questions as an
//! object keyed by id, each with a `criteria` field. Its response carries the
//! answers as an object keyed by the same ids. The binary question is a
//! Noul.

use crate::duplicates::triage::provider::{
    Answers, Provider, QuestionSet, offered_kind, wrong_type,
};
use crate::duplicates::triage::verdict::{
    DIVERGENCE_QUESTION, DecodeError, KIND_QUESTION, WORTH_QUESTION,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// The only place the API key is read from.
pub const API_KEY_VAR: &str = "TYPESAFE_API_KEY";
/// Overrides the API's base URL (tests point it at a local stub).
pub const BASE_URL_VAR: &str = "TYPESAFE_BASE_URL";
/// The API's base URL, absent an override.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// The evaluation endpoint, joined to the base URL.
pub const ENDPOINT_PATH: &str = "/v1/systemone";
/// The model asked when the configuration names none.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// The `TypeSafe` provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeSafe;

impl Provider for TypeSafe {
    fn id(&self) -> &'static str {
        "typesafe"
    }

    fn display_name(&self) -> &'static str {
        "TypeSafe API"
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

    /// None: `TypeSafe` was the only provider when the cache was keyed
    /// without one, so its keys stay exactly as they were and no existing
    /// cache is emptied.
    fn cache_namespace(&self) -> Option<&'static str> {
        None
    }

    fn encode(
        &self,
        state: Value,
        model: &str,
        questions: &QuestionSet,
    ) -> Value {
        json!({
            "state": state,
            "model": model,
            "questions": encode_questions(questions),
        })
    }

    fn decode(
        &self,
        body: &str,
    ) -> Result<Answers, DecodeError> {
        let response: Response = serde_json::from_str(body)
            .map_err(|e| DecodeError::new(format!("not a /v1/systemone response: {e}")))?;
        let (choice, kind_confidence) = response.choice(KIND_QUESTION)?;
        let kind = offered_kind(KIND_QUESTION, &choice)?;
        let (worth, worth_confidence) = response.score(WORTH_QUESTION)?;
        Ok(Answers {
            kind,
            kind_confidence,
            worth,
            worth_confidence,
            divergence: response.noul(DIVERGENCE_QUESTION)?,
        })
    }
}

/// The questions as an object keyed by id. A Choice's `criteria` maps each
/// option to its rubric, a Score's lists the level rubrics lowest first, and
/// a Noul's gives the rubric for true and for false.
fn encode_questions(set: &QuestionSet) -> Value {
    let kinds: Map<String, Value> = set
        .kind
        .options
        .iter()
        .map(|(kind, rubric)| (kind.wire_name().to_owned(), Value::from(*rubric)))
        .collect();
    let levels: Vec<&str> = set.worth.levels.iter().map(|(_, rubric)| *rubric).collect();
    let mut questions = Map::new();
    questions.insert(
        set.kind.id.to_owned(),
        json!({"type": "choice", "instructions": set.kind.instructions, "criteria": kinds}),
    );
    questions.insert(
        set.worth.id.to_owned(),
        json!({"type": "score", "instructions": set.worth.instructions, "criteria": levels}),
    );
    questions.insert(
        set.divergence.id.to_owned(),
        json!({
            "type": "noul",
            "instructions": set.divergence.instructions,
            "criteria": {"true": set.divergence.if_true, "false": set.divergence.if_false},
        }),
    );
    Value::Object(questions)
}

/// The parts of a `/v1/systemone` response this module reads. Answers stay
/// raw until asked for, so an answer nobody asked for is ignored and a
/// malformed one fails under its own question's name.
#[derive(Deserialize)]
struct Response {
    answers: HashMap<String, Value>,
}

/// One answer. Fields this module does not read (`probabilities`,
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
            .ok_or_else(|| DecodeError::new(format!("{id}: no answer")))?;
        Answer::deserialize(raw).map_err(|e| DecodeError::new(format!("{id}: {e}")))
    }

    /// The chosen option and the Choice's confidence.
    fn choice(
        &self,
        id: &str,
    ) -> Result<(String, f64), DecodeError> {
        match self.answer(id)? {
            Answer::Choice { choice, confidence } => Ok((choice, confidence)),
            other => Err(wrong_type(id, "choice", other.type_name())),
        }
    }

    /// The Score's position across the levels and its confidence.
    fn score(
        &self,
        id: &str,
    ) -> Result<(f64, f64), DecodeError> {
        match self.answer(id)? {
            Answer::Score { score, confidence } => Ok((score, confidence)),
            other => Err(wrong_type(id, "score", other.type_name())),
        }
    }

    fn noul(
        &self,
        id: &str,
    ) -> Result<f64, DecodeError> {
        match self.answer(id)? {
            Answer::Noul { noul } => Ok(noul),
            other => Err(wrong_type(id, "noul", other.type_name())),
        }
    }
}
