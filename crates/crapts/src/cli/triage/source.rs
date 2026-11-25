use crate::{FileAnalysis, Row, lines::LineIndex};
use cargo_crap_core::duplicates::{
    compare::DuplicatePair,
    model::Location,
    triage::{Excerpts, cache::CacheKey, provider::QuestionSet},
};
use cargo_crap_core::report::normalize_crapts_path;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io, ops::Range, path::PathBuf};

const NORMALIZATION_VERSION: u32 = 1;
const QUESTION_VERSION: u32 = 1;

pub(super) const QUESTIONS: QuestionSet = QuestionSet::new(
    "These JavaScript or TypeScript functions have similar syntax. What kind of duplication do their actual operations show?",
    "How much would sharing this JavaScript or TypeScript logic improve the code?",
    "If someone fixed a bug in one function, would the other need the same fix, and would they miss it?",
    [
        "Both perform the same steps on the same kinds of values and could share one implementation.",
        "Both use a familiar idiom such as an array pipeline or a promise chain for different jobs.",
        "A framework contract, event handler interface, lifecycle hook or generated binding dictates the common shape.",
        "One operation could serve both if it accepted a different value, callback or type parameter.",
    ],
    "Yes: both implement the same rule and neither points a maintainer to the other.",
    "No: the rule differs, or a shared caller or contract makes the second site easy to find.",
);

fn key(
    provider: &str,
    model: &str,
    first: &str,
    second: &str,
    normalization: u32,
    questions: u32,
) -> CacheKey<32> {
    let mut hash = Sha256::new();
    for field in [
        b"crapts-triage".as_slice(),
        &normalization.to_le_bytes(),
        &questions.to_le_bytes(),
        provider.as_bytes(),
        model.as_bytes(),
        first.as_bytes(),
        second.as_bytes(),
    ] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field);
    }
    CacheKey::from_digest(hash.finalize().into())
}

struct SourceFile<'a> {
    text: &'a str,
    lines: LineIndex,
    rows: HashMap<&'a str, &'a Row>,
}

pub(super) struct Sources<'a>(HashMap<String, SourceFile<'a>>);

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "duplicate location does not match its analyzed source",
    )
}

impl<'a> Sources<'a> {
    pub(super) fn new(
        files: &'a [FileAnalysis],
        snapshots: &'a HashMap<PathBuf, Vec<u8>>,
    ) -> io::Result<Self> {
        let mut sources = HashMap::new();
        for analysis in files {
            let Some(first) = analysis.rows.first() else {
                continue;
            };
            let bytes = snapshots.get(&first.file).ok_or_else(invalid)?;
            let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
            let rows = analysis.rows.iter().map(|r| (r.name.as_str(), r)).collect();
            sources.insert(
                normalize_crapts_path(&first.file),
                SourceFile {
                    text,
                    lines: LineIndex::new(text),
                    rows,
                },
            );
        }
        Ok(Self(sources))
    }

    fn side(
        &self,
        loc: &Location,
    ) -> io::Result<(String, String)> {
        let path = normalize_crapts_path(&loc.file);
        let file = self.0.get(&path).ok_or_else(invalid)?;
        let row = file.rows.get(loc.name.as_str()).ok_or_else(invalid)?;
        let regions = union(&row.excerpt_regions);
        let mut excerpts = Vec::new();
        let mut ranges = Vec::new();
        for region in regions {
            excerpts.push(
                file.text
                    .get(region.clone())
                    .ok_or_else(invalid)?
                    .replace("\r\n", "\n"),
            );
            let span = file
                .lines
                .span(oxc_span::Span::new(region.start as u32, region.end as u32));
            ranges.push(format!(
                "{}-{}",
                span.start.line,
                crate::duplicates::normalize::last_line(span)
            ));
        }
        Ok((excerpts.join("\n"), format!("{path}:{}", ranges.join(","))))
    }

    pub(super) fn pair(
        &self,
        pair: &DuplicatePair,
        provider: &str,
        model: &str,
    ) -> io::Result<Excerpts<32>> {
        let (first, a) = self.side(&pair.first)?;
        let (second, b) = self.side(&pair.second)?;
        let key = key(
            provider,
            model,
            &first,
            &second,
            NORMALIZATION_VERSION,
            QUESTION_VERSION,
        );
        Ok(Excerpts {
            first,
            second,
            key,
            locations: Some([a, b]),
        })
    }
}

fn union(regions: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::new();
    for region in regions {
        if let Some(previous) = merged.last_mut()
            && region.start < previous.end
        {
            previous.end = previous.end.max(region.end);
        } else {
            merged.push(region.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_bytes_are_copied_once_and_disjoint_slices_stay_separate() {
        assert_eq!(
            union(&[0..5, 2..3, 4..8, 8..10, 13..15]),
            [0..8, 8..10, 13..15]
        );
    }
    #[test]
    fn the_key_covers_order_field_boundaries_and_each_version() {
        let hash = |p, m, a, b, n, q| key(p, m, a, b, n, q).file_name();
        let first = hash("openai", "m", "ab", "c", 1, 2);
        for other in [
            hash("typesafe", "m", "ab", "c", 1, 2),
            hash("openai", "n", "ab", "c", 1, 2),
            hash("openai", "m", "c", "ab", 1, 2),
            hash("openai", "m", "a", "bc", 1, 2),
            hash("openai", "m", "ab", "c", 2, 2),
            hash("openai", "m", "ab", "c", 1, 3),
        ] {
            assert_ne!(first, other);
        }
        assert_eq!(
            first,
            "ce4f59fbff9ca55ae6c830e25b2d5809772c165ed0c5f26c987b0af03a2bfded.json"
        );
    }
}
