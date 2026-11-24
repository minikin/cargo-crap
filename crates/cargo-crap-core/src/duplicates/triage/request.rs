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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::triage::provider::{FIXTURE, Provider, typesafe::TypeSafe};
    use crate::duplicates::verdict::{WORTH_QUESTION, WorthExtracting};
    use std::path::PathBuf;
    fn questions() -> Value {
        TypeSafe.encode(json!({}), "", &FIXTURE)["questions"].clone()
    }
    fn located(
        file: &str,
        start_line: usize,
        end_line: usize,
        name: &str,
    ) -> Location {
        Location {
            file: PathBuf::from(file),
            start_line,
            end_line,
            name: name.to_owned(),
        }
    }

    fn pair() -> DuplicatePair {
        DuplicatePair {
            first: located("src/a.rs", 1, 5, "alpha"),
            second: located("src/b.rs", 10, 14, "beta"),
            score: 0.91,
        }
    }

    #[test]
    fn worth_levels_are_sent_in_label_order() {
        // The model answers with a position across the levels in the order
        // they were sent; the report labels that position with
        // `WorthExtracting::LABELS`. The two orders must be one order.
        let labels: Vec<&str> = WORTH_LEVELS.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, WorthExtracting::LABELS);
        let questions = questions();
        let sent: Vec<&str> = questions[WORTH_QUESTION]["criteria"]
            .as_array()
            .expect("ordered levels")
            .iter()
            .map(|level| level.as_str().expect("a rubric sentence"))
            .collect();
        let rubrics: Vec<&str> = WORTH_LEVELS.iter().map(|(_, rubric)| *rubric).collect();
        assert_eq!(sent, rubrics);
    }

    #[test]
    fn the_state_carries_both_sides_and_the_score() {
        let state = state(&pair(), "fn alpha() {}", "fn beta() {}");
        assert_eq!(state["function_a"]["name"], "alpha");
        assert_eq!(state["function_a"]["location"], "src/a.rs:1-5");
        assert_eq!(state["function_a"]["source"], "fn alpha() {}");
        assert_eq!(state["function_b"]["name"], "beta");
        assert_eq!(state["function_b"]["location"], "src/b.rs:10-14");
        assert_eq!(state["function_b"]["source"], "fn beta() {}");
        assert_eq!(state["structural_similarity"], 0.91);
        assert_eq!(state.as_object().expect("a map").len(), 3, "nothing else");
    }
}
