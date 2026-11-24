//! Assemble the state shared by all duplication questions.

use crate::duplicates::{compare::DuplicatePair, model::Location};
use serde_json::{Value, json};

/// The worth-extracting levels in the order they are sent, each beside the
/// label the report prints for it. One table, so the two orders cannot drift
/// apart.
pub const WORTH_LEVELS: [(&str, &str); 4] = [
    (
        "leave-it",
        "The two do different jobs, or their shape is imposed from outside; one shared \
         function would couple code that changes for different reasons.",
    ),
    (
        "optional",
        "The two share a few lines of real logic, but it is short and stable, so a shared \
         function would save little and cost an extra indirection.",
    ),
    (
        "worthwhile",
        "The two share a substantial block of logic that is likely to change; a shared \
         function would remove real repetition.",
    ),
    (
        "should-be-one",
        "The two are the same routine in all but name; keeping both means every change has \
         to be made twice.",
    ),
];

/// What the model judges: each side's name, location and source, plus the
/// similarity score that made them a pair.
#[must_use]
pub fn state(
    pair: &DuplicatePair,
    source_a: &str,
    source_b: &str,
) -> Value {
    json!({
        "function_a": side(&pair.first, source_a),
        "function_b": side(&pair.second, source_b),
        "structural_similarity": pair.score,
    })
}

fn side(
    at: &Location,
    source: &str,
) -> Value {
    json!({
        "name": at.name,
        "location": format!("{}:{}-{}", at.file.display(), at.start_line, at.end_line),
        "source": source,
    })
}
