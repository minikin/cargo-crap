pub use cargo_crap_core::duplicates::triage::provider::typesafe::{
    API_KEY_VAR, BASE_URL_VAR, DEFAULT_BASE_URL, DEFAULT_MODEL, ENDPOINT_PATH, TypeSafe,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::compare::DuplicatePair;
    use crate::duplicates::extract::Location;
    use crate::duplicates::triage::provider::Provider;
    use crate::duplicates::triage::request::{self, QUESTIONS};
    use proptest::prelude::*;
    use std::path::PathBuf;

    /// The request builder as it was before the seam, pasted unchanged: the
    /// oracle the seam's `TypeSafe` encoding must equal.
    mod before_seam {
        use crate::duplicates::compare::DuplicatePair;
        use crate::duplicates::extract::Location;
        use crate::duplicates::triage::verdict::{
            DIVERGENCE_QUESTION, KIND_QUESTION, Kind, WORTH_QUESTION,
        };
        use serde_json::{Map, Value, json};

        const WORTH_LEVELS: [(&str, &str); 4] = [
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

        const KIND_INSTRUCTIONS: &str = "`function_a` and `function_b` are two Rust functions that a \
             structural clone detector flagged as similar; `structural_similarity` is its score from 0 \
             to 1. What kind of duplication is this?";

        const WORTH_INSTRUCTIONS: &str = "How much would merging `function_a` and `function_b` into one shared function improve \
             this codebase?";

        const DIVERGENCE_INSTRUCTIONS: &str = "If a bug were fixed in `function_a`, would the same fix \
             likely be needed in `function_b` and be missed there?";

        /// The request body: the pair's state, the model and the three questions.
        #[must_use]
        pub fn body(
            pair: &DuplicatePair,
            source_a: &str,
            source_b: &str,
            model: &str,
        ) -> Value {
            json!({
                "state": state(pair, source_a, source_b),
                "model": model,
                "questions": questions(),
            })
        }

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

        /// The three questions, keyed by the ids the verdict decodes.
        #[must_use]
        pub fn questions() -> Value {
            let kinds: Map<String, Value> = Kind::ALL
                .into_iter()
                .map(|kind| (kind.wire_name().to_owned(), Value::from(kind_rubric(kind))))
                .collect();
            let levels: Vec<&str> = WORTH_LEVELS.iter().map(|(_, rubric)| *rubric).collect();
            let mut questions = Map::new();
            questions.insert(
                KIND_QUESTION.to_owned(),
                json!({"type": "choice", "instructions": KIND_INSTRUCTIONS, "criteria": kinds}),
            );
            questions.insert(
                WORTH_QUESTION.to_owned(),
                json!({"type": "score", "instructions": WORTH_INSTRUCTIONS, "criteria": levels}),
            );
            questions.insert(
                DIVERGENCE_QUESTION.to_owned(),
                json!({
                    "type": "noul",
                    "instructions": DIVERGENCE_INSTRUCTIONS,
                    "criteria": {
                        "true": "Yes: the two share logic a fix would have to change in both, and \
                                 nothing about them — the same file, a shared name, a common caller — \
                                 would lead someone fixing one to the other.",
                        "false": "No: a fix in one would not apply to the other, or anyone fixing one \
                                  would plainly see the other needs it too.",
                    },
                }),
            );
            Value::Object(questions)
        }

        /// The rubric sentence the model reads for each kind.
        fn kind_rubric(kind: Kind) -> &'static str {
            match kind {
                Kind::SameLogic => {
                    "One routine written twice: the same steps on the same kinds of values, differing \
                     only in the names of functions and variables. Either could replace the other as \
                     it is, without adding a parameter."
                },
                Kind::SharedShapeOnly => {
                    "The similarity is only a shared Rust idiom or a run of similar calls, such as a \
                     sequence of `writeln!` or builder calls; the two do different jobs and there is \
                     nothing to consolidate."
                },
                Kind::StructuralObligation => {
                    "The shared shape is imposed from outside — a trait's method, a visitor, a macro or \
                     a framework contract — so it cannot be shared away."
                },
                Kind::Parameterisable => {
                    "The same routine except for something that genuinely differs between them — a \
                     constant, a type, a field, or one step — so merging them needs a new parameter, \
                     closure or generic."
                },
            }
        }
    }

    fn located(
        file: String,
        name: String,
    ) -> Location {
        Location {
            file: PathBuf::from(file),
            start_line: 1,
            end_line: 9,
            name,
        }
    }

    proptest! {
        /// The seam sends `TypeSafe` exactly the body it was sent before the seam.
        #[test]
        fn the_body_through_the_seam_is_the_body_sent_before_it(
            file_a in "[a-z/]{1,20}\\.rs",
            file_b in "[a-z/]{1,20}\\.rs",
            name_a in "[a-z_]{1,12}",
            name_b in "[a-z_]{1,12}",
            source_a in ".{0,80}",
            source_b in ".{0,80}",
            score in 0.0..=1.0f64,
            model in "[a-z0-9.-]{1,16}",
        ) {
            let pair = DuplicatePair {
                first: located(file_a, name_a),
                second: located(file_b, name_b),
                score,
            };
            let through_seam = TypeSafe.encode(
                request::state(&pair, &source_a, &source_b),
                &model,
                &QUESTIONS,
            );
            let before = before_seam::body(&pair, &source_a, &source_b, &model);
            prop_assert_eq!(through_seam.to_string(), before.to_string());
        }

    }
}
