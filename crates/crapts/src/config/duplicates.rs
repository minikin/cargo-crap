use super::InputError;
use cargo_crap_core::duplicates::triage::provider::{self, Provider};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct DuplicatesConfig {
    pub enabled: Option<bool>,
    #[serde(alias = "min_similarity", alias = "threshold")]
    pub min_similarity: Option<f64>,
    #[serde(alias = "min_nodes")]
    pub min_nodes: Option<usize>,
    #[serde(default)]
    pub triage: TriageConfig,
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct TriageConfig {
    pub enabled: Option<bool>,
    pub provider: Option<ProviderChoice>,
    pub model: Option<String>,
    #[serde(alias = "confidence_floor")]
    pub confidence_floor: Option<f64>,
    #[serde(alias = "max_requests")]
    pub max_requests: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub struct ProviderChoice {
    pub id: &'static str,
    pub default_model: &'static str,
}

impl From<&dyn Provider> for ProviderChoice {
    fn from(provider: &dyn Provider) -> Self {
        Self {
            id: provider.id(),
            default_model: provider.default_model(),
        }
    }
}

impl<'de> Deserialize<'de> for ProviderChoice {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        provider::by_id(&id).map(Self::from).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown duplicates.triage.provider {id}"))
        })
    }
}

impl DuplicatesConfig {
    pub(super) fn validate(
        &mut self,
        path: &Path,
    ) -> Result<(), InputError> {
        for (key, value) in [
            ("duplicates.min-similarity", &mut self.min_similarity),
            (
                "duplicates.triage.confidence-floor",
                &mut self.triage.confidence_floor,
            ),
        ] {
            let Some(n) = value else { continue };
            if !(0.0..=1.0).contains(n) {
                return Err(InputError::at(
                    path,
                    format!("{key} must be between 0 and 1, got {n}"),
                ));
            }
            *n += 0.0;
        }
        if self
            .triage
            .model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty())
        {
            return Err(InputError::at(
                path,
                "duplicates.triage.model must not be blank",
            ));
        }
        Ok(())
    }
}
