pub use cargo_crap_core::duplicates::triage::provider::typesafe::{
    API_KEY_VAR, BASE_URL_VAR, DEFAULT_BASE_URL, DEFAULT_MODEL, ENDPOINT_PATH, TypeSafe,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::compare::DuplicatePair;
    use crate::duplicates::extract::Location;
    use crate::duplicates::triage::provider::{Answers, Provider};
    use crate::duplicates::triage::request::{self, QUESTIONS};
    use crate::duplicates::triage::verdict::{Kind, Verdict};
    use crate::duplicates::verdict::{DIVERGENCE_QUESTION, KIND_QUESTION, WORTH_QUESTION};
    use proptest::prelude::*;
    use serde_json::{Map, json};
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

    /// A response carrying `answers` under the ids the encoded questions use.
    fn response_for(answers: &Answers) -> String {
        let questions = TypeSafe.encode(json!({}), "jev", &QUESTIONS)["questions"].clone();
        let ids: Vec<&String> = questions.as_object().expect("keyed by id").keys().collect();
        let mut by_id = Map::new();
        for id in ids {
            let answer = match questions[id]["type"].as_str().expect("a type") {
                "choice" => json!({
                    "type": "choice",
                    "choice": answers.kind.wire_name(),
                    "confidence": answers.kind_confidence,
                }),
                "score" => json!({
                    "type": "score",
                    "score": answers.worth,
                    "confidence": answers.worth_confidence,
                }),
                other => {
                    assert_eq!(other, "noul");
                    json!({"type": "noul", "noul": answers.divergence})
                },
            };
            by_id.insert(id.clone(), answer);
        }
        json!({"model": "jev", "answers": by_id, "usage": {}}).to_string()
    }

    /// A full `/v1/systemone` response with the three answers, overridable
    /// one answer at a time.
    fn response(
        kind: &str,
        worth: &str,
        divergence: &str,
    ) -> String {
        format!(
            r#"{{"model":"jev-1.13.0","answers":{{"duplication_kind":{kind},"worth_extracting":{worth},"divergence_risk":{divergence}}},"usage":{{"input_tokens":900,"output_tokens":40}}}}"#
        )
    }

    const KIND: &str = r#"{"type":"choice","choice":"same_logic","probabilities":{"same_logic":0.86,"shared_shape_only":0.08,"structural_obligation":0.04,"parameterisable":0.02},"confidence":0.81}"#;
    const WORTH: &str = r#"{"type":"score","score":2.4,"legend":{"0":"a","1":"b","2":"c","3":"d"},"probabilities":{"0":0.0,"1":0.1,"2":0.4,"3":0.5},"confidence":0.62}"#;
    const DIVERGENCE: &str = r#"{"type":"noul","noul":0.8}"#;

    #[test]
    fn typesafe_names_its_api_and_environment() {
        let provider = TypeSafe;
        assert_eq!(provider.id(), "typesafe");
        assert_eq!(provider.display_name(), "TypeSafe API");
        assert_eq!(provider.key_var(), "TYPESAFE_API_KEY");
        assert_eq!(provider.base_url_var(), "TYPESAFE_BASE_URL");
        assert_eq!(provider.default_base_url(), "https://api.typesafe.ai");
        assert_eq!(provider.endpoint_path(), "/v1/systemone");
        assert_eq!(provider.default_model(), "jev-latest");
    }

    #[test]
    fn decodes_a_canned_response() {
        let answers = TypeSafe
            .decode(&response(KIND, WORTH, DIVERGENCE))
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
    fn a_missing_answer_is_an_error_naming_the_question() {
        let body = r#"{"model":"jev","answers":{},"usage":{}}"#;
        let err = TypeSafe.decode(body).expect_err("nothing to decode");
        assert!(err.to_string().contains(KIND_QUESTION), "{err}");
        let without_worth = response(KIND, WORTH, DIVERGENCE)
            .replace(&format!(r#","worth_extracting":{WORTH}"#), "");
        let err = TypeSafe
            .decode(&without_worth)
            .expect_err("no worth answer");
        assert!(err.to_string().contains(WORTH_QUESTION), "{err}");
        let without_divergence = response(KIND, WORTH, DIVERGENCE)
            .replace(&format!(r#","divergence_risk":{DIVERGENCE}"#), "");
        let err = TypeSafe
            .decode(&without_divergence)
            .expect_err("no divergence answer");
        assert!(err.to_string().contains(DIVERGENCE_QUESTION), "{err}");
    }

    #[test]
    fn an_answer_of_the_wrong_type_is_an_error_naming_both_types() {
        // The message names the question, the type it needed and the type
        // that came back: the three facts a mismatched question set needs.
        let cases = [
            (
                response(DIVERGENCE, WORTH, DIVERGENCE),
                KIND_QUESTION,
                "a choice answer, got noul",
            ),
            (
                response(KIND, KIND, DIVERGENCE),
                WORTH_QUESTION,
                "a score answer, got choice",
            ),
            (
                response(KIND, WORTH, WORTH),
                DIVERGENCE_QUESTION,
                "a noul answer, got score",
            ),
        ];
        for (body, question, types) in cases {
            let err = TypeSafe
                .decode(&body)
                .expect_err("wrong answer type")
                .to_string();
            assert!(err.contains(question) && err.contains(types), "{err}");
        }
    }

    #[test]
    fn an_option_that_was_never_offered_is_an_error() {
        let kind = r#"{"type":"choice","choice":"copy_paste","probabilities":{},"confidence":0.9}"#;
        let err = TypeSafe
            .decode(&response(kind, WORTH, DIVERGENCE))
            .expect_err("unknown option")
            .to_string();
        assert_eq!(
            err,
            r#"duplication_kind: "copy_paste" is not an offered option"#
        );
    }

    #[test]
    fn a_body_that_is_not_a_response_is_an_error() {
        let err = TypeSafe
            .decode("not json")
            .expect_err("not JSON")
            .to_string();
        assert!(err.starts_with("not a /v1/systemone response: "), "{err}");
        assert!(TypeSafe.decode(r#"{"error":"overloaded"}"#).is_err());
    }

    #[test]
    fn an_answer_nobody_asked_for_is_ignored() {
        let body = response(KIND, WORTH, DIVERGENCE).replace(
            r#""answers":{"#,
            r#""answers":{"diagnostic":{"type":"trace","id":"x"},"#,
        );
        let answers = TypeSafe
            .decode(&body)
            .expect("the three answers still decode");
        assert_eq!(answers.kind, Kind::SameLogic);
    }

    #[test]
    fn a_malformed_answer_names_its_question() {
        let kind = r#"{"type":"choice","choice":"same_logic"}"#;
        let err = TypeSafe
            .decode(&response(kind, WORTH, DIVERGENCE))
            .expect_err("a choice without confidence")
            .to_string();
        assert!(
            err.contains(KIND_QUESTION) && err.contains("confidence"),
            "{err}"
        );

        let divergence = r#"{"type":"verdict","noul":0.8}"#;
        let err = TypeSafe
            .decode(&response(KIND, WORTH, divergence))
            .expect_err("an unknown answer type")
            .to_string();
        assert!(err.contains(DIVERGENCE_QUESTION), "{err}");
    }

    #[test]
    fn an_answer_naming_any_offered_option_decodes() {
        let body = TypeSafe.encode(json!({}), "jev", &QUESTIONS);
        let offered = body["questions"][KIND_QUESTION]["criteria"]
            .as_object()
            .expect("option -> rubric");
        assert_eq!(offered.len(), Kind::ALL.len(), "every kind is offered");
        for option in offered.keys() {
            let kind = Kind::from_wire(option).expect("an offered option is a kind");
            let answers = Answers {
                kind,
                kind_confidence: 0.9,
                worth: 1.0,
                worth_confidence: 0.9,
                divergence: 0.5,
            };
            let decoded = TypeSafe.decode(&response_for(&answers)).expect("decodes");
            assert_eq!(decoded.kind.wire_name(), option);
        }
    }

    #[test]
    fn the_body_names_the_model_and_carries_state_and_questions() {
        let state = json!({"function_a": {"name": "alpha"}});
        let body = TypeSafe.encode(state.clone(), "jev-latest", &QUESTIONS);
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], state);
        assert_eq!(body["questions"], before_seam::questions());
        assert_eq!(body.as_object().expect("a map").len(), 3);
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

        /// Every question encoded is answered by the id decode reads, and
        /// every answer it reads comes back as sent.
        #[test]
        fn encode_and_decode_agree(answers in answers()) {
            let decoded = TypeSafe.decode(&response_for(&answers)).expect("decodes");
            prop_assert_eq!(decoded, answers);
            prop_assert!(Verdict::from_answers(decoded).is_ok());
        }
    }
}
