use super::{is_valid_crapts_path, is_valid_fingerprint, stem};
use crate::duplicates::{
    model::{DuplicateContext, Location},
    verdict::{Kind, WorthExtracting},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Deserializer, de};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
    path::Path,
};

/// The producer's original coordinates and the complete families represented in its groups.
#[derive(Debug, Clone)]
pub struct DuplicateHistory {
    pub groups: Vec<Vec<Location>>,
    pub context: DuplicateContext,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn fraction<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(d)?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(de::Error::custom("expected a fraction between 0 and 1"))
    }
}

fn file<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let path = String::deserialize(d)?;
    if is_valid_crapts_path(&path) {
        Ok(path)
    } else {
        Err(de::Error::custom(format!("invalid duplicate path {path}")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    #[serde(deserialize_with = "file", rename = "first_file")]
    _first_file: String,
    #[serde(rename = "first_function")]
    _first_function: String,
    #[serde(rename = "first_start_line")]
    _first_start: NonZeroUsize,
    #[serde(rename = "first_end_line")]
    _first_end: NonZeroUsize,
    #[serde(deserialize_with = "file", rename = "second_file")]
    _second_file: String,
    #[serde(rename = "second_function")]
    _second_function: String,
    #[serde(rename = "second_start_line")]
    _second_start: NonZeroUsize,
    #[serde(rename = "second_end_line")]
    _second_end: NonZeroUsize,
    #[serde(rename = "score", deserialize_with = "fraction")]
    _score: f64,
    #[serde(default, deserialize_with = "present")]
    triage: Option<Triage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    #[serde(deserialize_with = "file")]
    file: String,
    function: String,
    start_line: NonZeroUsize,
    end_line: NonZeroUsize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    members: Vec<Member>,
    #[serde(rename = "score", deserialize_with = "fraction")]
    _score: f64,
    #[serde(default, deserialize_with = "present")]
    triage: Option<Triage>,
    #[serde(default, deserialize_with = "present")]
    status: Option<Status>,
    #[serde(default, deserialize_with = "present")]
    grown_from: Option<usize>,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Status {
    New,
    Grown,
    Unchanged,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Triage {
    kind: String,
    #[serde(rename = "confidence", deserialize_with = "fraction")]
    _confidence: f64,
    #[serde(default, deserialize_with = "present")]
    worth_extracting: Option<String>,
    #[serde(default, deserialize_with = "present")]
    worth_extracting_score: Option<f64>,
    #[serde(default, deserialize_with = "present")]
    divergence_risk: Option<f64>,
}
impl Triage {
    fn validate(&self) -> Result<()> {
        match (
            &self.worth_extracting,
            self.worth_extracting_score,
            self.divergence_risk,
        ) {
            (None, None, None) => ensure!(
                self.kind == "uncertain",
                "missing fields for assessed duplicate triage"
            ),
            (Some(label), Some(worth), Some(risk)) => {
                ensure!(
                    Kind::ALL.iter().any(|k| k.label() == self.kind),
                    "invalid duplicate triage kind"
                );
                ensure!(
                    WorthExtracting::LABELS.contains(&label.as_str()),
                    "invalid worth_extracting"
                );
                ensure!(
                    (0.0..=3.0).contains(&worth) && (0.0..=1.0).contains(&risk),
                    "invalid duplicate triage score"
                );
            },
            _ => anyhow::bail!("incomplete duplicate triage"),
        }
        Ok(())
    }
}

impl Group {
    fn locations(
        self,
        delta: bool,
    ) -> Result<Vec<Location>> {
        ensure!(
            self.members.len() >= 2,
            "a duplicate group requires at least two members"
        );
        ensure!(
            delta || self.status.is_none(),
            "duplicate group status on an absolute report"
        );
        match self.status {
            Some(Status::Grown) => ensure!(
                self.grown_from.is_some_and(|n| n >= 2),
                "invalid grown_from"
            ),
            _ => ensure!(self.grown_from.is_none(), "unexpected grown_from"),
        }
        if let Some(t) = self.triage {
            t.validate()?;
        }
        Ok(self
            .members
            .into_iter()
            .map(|m| Location {
                file: m.file.into(),
                name: m.function,
                start_line: m.start_line.get(),
                end_line: m.end_line.get(),
            })
            .collect())
    }
}

fn families(
    context: &DuplicateContext,
    groups: &[Vec<Location>],
) -> Result<()> {
    let mut index = BTreeMap::new();
    for family in &context.families {
        ensure!(
            is_valid_crapts_path(&family.file),
            "invalid duplicate family path"
        );
        ensure!(
            !family.fingerprints.is_empty()
                && family.fingerprints.iter().all(|s| is_valid_fingerprint(s)),
            "invalid duplicate family fingerprints"
        );
        let key = (family.file.as_str(), family.stem.as_str());
        ensure!(
            index.insert(key, family.fingerprints.len()).is_none(),
            "duplicate family record"
        );
    }
    let mut used = BTreeSet::new();
    for member in groups.iter().flatten() {
        let name = stem(&member.name);
        let key = (member.file.to_str().unwrap_or_default(), name);
        let count = index.get(&key).context("missing duplicate family")?;
        if name != member.name {
            let ordinal: usize = member.name[name.len() + 1..].parse()?;
            ensure!(
                (2..=*count).contains(&ordinal),
                "duplicate ordinal outside family"
            );
        }
        used.insert(key);
    }
    ensure!(used.len() == index.len(), "unreferenced duplicate family");
    Ok(())
}

fn valid_prefix(prefix: &str) -> bool {
    prefix.is_empty()
        || (is_valid_crapts_path(prefix)
            && prefix.split('/').all(|c| !matches!(c, "" | "." | "..")))
}

pub(super) fn read(
    path: &Path,
    values: [Option<&RawValue>; 4],
    delta: bool,
) -> Result<Option<DuplicateHistory>> {
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let [Some(pairs), Some(groups), Some(context), skipped] = values else {
        anyhow::bail!("baseline {}: incomplete duplicate record", path.display())
    };
    let decode = || -> Result<DuplicateHistory> {
        if let Some(n) = skipped {
            serde_json::from_str::<NonZeroUsize>(n.get())?;
        }
        for p in serde_json::from_str::<Vec<Pair>>(pairs.get())? {
            if let Some(t) = p.triage {
                t.validate()?;
            }
        }
        let groups = serde_json::from_str::<Vec<Group>>(groups.get())?
            .into_iter()
            .map(|g| g.locations(delta))
            .collect::<Result<Vec<_>>>()?;
        let context: DuplicateContext = serde_json::from_str(context.get())?;
        ensure!(
            context.repo_prefix.as_deref().is_none_or(valid_prefix),
            "invalid duplicate repo_prefix"
        );
        families(&context, &groups)?;
        Ok(DuplicateHistory { groups, context })
    };
    decode()
        .map(Some)
        .with_context(|| format!("baseline {}: invalid duplicate record", path.display()))
}
