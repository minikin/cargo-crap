//! Who triage asks: one trait every API implements, and the registry of
//! the APIs this build knows.
//!
//! A provider names its endpoint and environment, turns the neutral
//! [`QuestionSet`] into its own request body, and decodes its own response
//! back into [`Answers`]. Everything else (the HTTP client and its retries,
//! the cache, the confidence floor, the rendering) is shared, so adding a
//! provider is one file implementing [`Provider`] and one entry in
//! [`PROVIDERS`].
//!
//! Compiled in every build: configuration validation lists the registered
//! ids whether or not the `triage` feature put a client in.

pub mod openai;
pub mod typesafe;

use crate::duplicates::triage::verdict::{DecodeError, Kind};
use serde_json::Value;

/// One API that can answer the triage questions.
pub trait Provider: Sync {
    /// The name configuration selects it by.
    fn id(&self) -> &'static str;
    /// How warnings and errors name the API.
    fn display_name(&self) -> &'static str;
    /// The environment variable the API key is read from, and from nowhere
    /// else.
    fn key_var(&self) -> &'static str;
    /// The environment variable that overrides the base URL.
    fn base_url_var(&self) -> &'static str;
    /// The base URL absent an override.
    fn default_base_url(&self) -> &'static str;
    /// The path joined to the base URL to reach the endpoint.
    fn endpoint_path(&self) -> &'static str;
    /// The model asked when the configuration names none.
    fn default_model(&self) -> &'static str;
    /// What the verdict cache mixes into a key so this provider's verdicts
    /// are never served for another's. Its id, unless it overrides this.
    fn cache_namespace(&self) -> Option<&'static str> {
        Some(self.id())
    }
    /// The request body asking `questions` about `state` of `model`.
    fn encode(
        &self,
        state: Value,
        model: &str,
        questions: &QuestionSet,
    ) -> Value;
    /// The three answers in a response body.
    ///
    /// # Errors
    ///
    /// When the body is not a response, an answer is missing or of the wrong
    /// type, or the chosen value names no [`Kind`]. Every kind is offered,
    /// so a value naming a kind names an offered one. Ranges are not checked
    /// here: [`Verdict::from_answers`] checks them for every provider.
    ///
    /// [`Verdict::from_answers`]: crate::duplicates::triage::verdict::Verdict::from_answers
    fn decode(
        &self,
        body: &str,
    ) -> Result<Answers, DecodeError>;
}

/// The three questions asked about every pair, as data each provider
/// translates into its own wire shape and never rewords.
#[derive(Debug, Clone, Copy)]
pub struct QuestionSet {
    pub kind: ChoiceQuestion,
    pub worth: ScoreQuestion,
    pub divergence: BinaryQuestion,
}

/// A question answered by picking one of several options.
#[derive(Debug, Clone, Copy)]
pub struct ChoiceQuestion {
    pub id: &'static str,
    pub instructions: &'static str,
    /// Each option and the rubric the model reads for it.
    pub options: &'static [(Kind, &'static str)],
}

/// A question answered by a position across ordered levels.
#[derive(Debug, Clone, Copy)]
pub struct ScoreQuestion {
    pub id: &'static str,
    pub instructions: &'static str,
    /// Each level's label and rubric, lowest first.
    pub levels: &'static [(&'static str, &'static str)],
}

/// A question answered by the probability that a statement holds.
#[derive(Debug, Clone, Copy)]
pub struct BinaryQuestion {
    pub id: &'static str,
    pub instructions: &'static str,
    /// What "true" means.
    pub if_true: &'static str,
    /// What "false" means.
    pub if_false: &'static str,
}

/// The three answers as a provider decoded them, before any range check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Answers {
    /// The chosen kind.
    pub kind: Kind,
    /// The kind Choice's confidence.
    pub kind_confidence: f64,
    /// The worth Score's position across its levels, the lowest being 0.
    pub worth: f64,
    /// The worth Score's confidence.
    pub worth_confidence: f64,
    /// The probability that a fix to one side would be missed in the other.
    /// Neither API returns a confidence for it.
    pub divergence: f64,
}

/// The kind a choice answer to `question` names, or an error naming the
/// question and the value when it names none.
pub(crate) fn offered_kind(
    question: &str,
    choice: &str,
) -> Result<Kind, DecodeError> {
    Kind::from_wire(choice)
        .ok_or_else(|| DecodeError::new(format!("{question}: {choice:?} is not an offered option")))
}

/// The error for an answer to `question` that came back as `got` where
/// `expected` was asked.
pub(crate) fn wrong_type(
    question: &str,
    expected: &str,
    got: &str,
) -> DecodeError {
    DecodeError::new(format!(
        "{question}: expected a {expected} answer, got {got}"
    ))
}

static TYPESAFE: typesafe::TypeSafe = typesafe::TypeSafe;
static OPENAI: openai::OpenAi = openai::OpenAi;

/// Every provider this build can ask.
pub static PROVIDERS: &[&dyn Provider] = &[&TYPESAFE, &OPENAI];

/// The provider asked when the configuration names none.
#[must_use]
pub fn default_provider() -> &'static dyn Provider {
    &TYPESAFE
}

/// The provider registered as `id`, matched exactly.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static dyn Provider> {
    PROVIDERS
        .iter()
        .copied()
        .find(|provider| provider.id() == id)
}

/// Every registered id, in registry order.
pub fn ids() -> impl Iterator<Item = &'static str> {
    PROVIDERS.iter().map(|provider| provider.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn typesafe_is_registered_and_is_the_default() {
        let provider = by_id("typesafe").expect("registered");
        assert_eq!(provider.id(), "typesafe");
        assert_eq!(default_provider().id(), "typesafe");
    }

    #[test]
    fn a_provider_namespaces_its_cache_by_its_id_unless_it_says_otherwise() {
        let openai = by_id("openai").expect("registered");
        assert_eq!(openai.cache_namespace(), Some("openai"));
        assert_eq!(default_provider().cache_namespace(), None);
    }

    #[test]
    fn openai_is_registered() {
        let provider = by_id("openai").expect("registered");
        assert_eq!(provider.id(), "openai");
        assert!(ids().any(|id| id == "openai"));
    }

    #[test]
    fn an_unknown_id_names_no_provider() {
        assert!(by_id("acme").is_none());
        assert!(by_id("").is_none());
        assert!(by_id("TypeSafe").is_none(), "ids are matched exactly");
    }

    #[test]
    fn registered_ids_are_unique_and_each_finds_itself() {
        let listed: Vec<&str> = ids().collect();
        let unique: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(unique.len(), listed.len(), "{listed:?}");
        assert!(listed.contains(&"typesafe"), "{listed:?}");
        for id in listed {
            assert_eq!(by_id(id).map(Provider::id), Some(id));
        }
    }
}
