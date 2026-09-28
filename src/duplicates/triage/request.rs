//! The request sent for one pair: its state and the three questions.
//!
//! One request per pair, carrying that pair's two function bodies and
//! nothing else. The three questions share the state and are evaluated
//! independently; their ids and option names are the ones
//! [`verdict`](super::verdict) decodes.

use crate::duplicates::compare::DuplicatePair;
use crate::duplicates::extract::Location;
use crate::duplicates::triage::verdict::{
    DIVERGENCE_QUESTION, KIND_QUESTION, Kind, WORTH_QUESTION,
};
use serde_json::{Map, Value, json};
use std::io;
use std::path::Path;

/// The version of the question set below. Bump it whenever anything changes
/// what the model is asked — a question's wording, options or levels, or how
/// the state's similarity score is computed: it is part of the cache key, so
/// a verdict never outlives the question that produced it.
pub const QUESTION_SET_VERSION: u32 = 1;

/// The worth-extracting levels in the order they are sent, each beside the
/// label the report prints for it — one table, so the two orders cannot
/// drift apart.
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

const KIND_INSTRUCTIONS: &str = "`function_a` and `function_b` are two Rust functions that a \
     structural clone detector flagged as similar; `structural_similarity` is its score from 0 \
     to 1. What kind of duplication is this?";

const WORTH_INSTRUCTIONS: &str = "How much would merging `function_a` and `function_b` into one shared function improve \
     this codebase?";

const DIVERGENCE_INSTRUCTIONS: &str = "If a bug were fixed in `function_a`, would the same fix \
     likely be needed in `function_b` and be missed there?";

/// The request body for `pair`, with both sides' source read back from disk.
///
/// # Errors
///
/// When either side's file cannot be read, or no longer holds the lines the
/// scan located the function on.
pub fn build(
    pair: &DuplicatePair,
    model: &str,
) -> io::Result<Value> {
    let (source_a, source_b) = sources(pair)?;
    Ok(body(pair, &source_a, &source_b, model))
}

/// Both sides' source, read back from disk — what the request carries and
/// what the cache key covers.
///
/// # Errors
///
/// When either side's file cannot be read, or no longer holds the lines the
/// scan located the function on.
pub fn sources(pair: &DuplicatePair) -> io::Result<(String, String)> {
    Ok((read_location(&pair.first)?, read_location(&pair.second)?))
}

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

/// What the model judges: both sides — name, location, source — and the
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

fn read_location(at: &Location) -> io::Result<String> {
    read_span(&at.file, at.start_line, at.end_line)
}

/// Lines `start_line..=end_line` (1-indexed, inclusive) of `file` and
/// nothing else, joined by `\n`. CRLF endings come back as `\n` on purpose:
/// a CRLF and an LF checkout of the same code send the same source, and so
/// share cached verdicts.
///
/// # Errors
///
/// When the file cannot be read, or the range is empty or not within it —
/// which, for a range the scan produced, means the file changed since.
pub fn read_span(
    file: &Path,
    start_line: usize,
    end_line: usize,
) -> io::Result<String> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", file.display())))?;
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || start_line > end_line || end_line > lines.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: lines {start_line}-{end_line} are not in the file ({} lines); it changed \
                 after it was scanned",
                file.display(),
                lines.len()
            ),
        ));
    }
    Ok(lines[start_line - 1..end_line].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplicates::extract::Location;
    use crate::duplicates::triage::verdict::{
        DIVERGENCE_QUESTION, KIND_QUESTION, Kind, Verdict, WORTH_QUESTION, WorthExtracting,
    };
    use proptest::prelude::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

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
    fn question_ids_and_types_match_what_the_verdict_decodes() {
        let questions = questions();
        assert_eq!(questions[KIND_QUESTION]["type"], "choice");
        assert_eq!(questions[WORTH_QUESTION]["type"], "score");
        assert_eq!(questions[DIVERGENCE_QUESTION]["type"], "noul");
        assert_eq!(
            questions.as_object().expect("a map").len(),
            3,
            "exactly the three questions"
        );
        let offered: BTreeSet<&str> = questions[KIND_QUESTION]["criteria"]
            .as_object()
            .expect("option -> rubric")
            .keys()
            .map(String::as_str)
            .collect();
        let decodable: BTreeSet<&str> = Kind::ALL.iter().map(|k| k.wire_name()).collect();
        assert_eq!(
            offered, decodable,
            "every offered option decodes, and only those"
        );
    }

    #[test]
    fn every_kind_is_offered_with_its_own_rubric() {
        // The rubric is what the model reads to tell the kinds apart: four
        // options sharing one description, or none, would leave it guessing.
        let questions = questions();
        let rubrics: Vec<&str> = questions[KIND_QUESTION]["criteria"]
            .as_object()
            .expect("option -> rubric")
            .values()
            .map(|rubric| rubric.as_str().expect("a rubric sentence"))
            .collect();
        let distinct: BTreeSet<&str> = rubrics.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            Kind::ALL.len(),
            "one rubric per kind: {rubrics:?}"
        );
        assert!(
            rubrics.iter().all(|r| r.split_whitespace().count() >= 8),
            "each rubric is a sentence: {rubrics:?}"
        );
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
    fn an_answer_naming_any_offered_option_decodes() {
        for option in questions()[KIND_QUESTION]["criteria"]
            .as_object()
            .expect("option -> rubric")
            .keys()
        {
            let body = serde_json::json!({
                "model": "jev",
                "answers": {
                    KIND_QUESTION: {"type": "choice", "choice": option, "confidence": 0.9},
                    WORTH_QUESTION: {"type": "score", "score": 1.0, "confidence": 0.9},
                    DIVERGENCE_QUESTION: {"type": "noul", "noul": 0.5},
                },
                "usage": {}
            });
            let verdict = Verdict::decode(&body.to_string()).expect("decodes");
            assert_eq!(verdict.kind.wire_name(), option);
        }
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

    #[test]
    fn the_body_names_the_model_and_carries_state_and_questions() {
        let body = body(&pair(), "fn alpha() {}", "fn beta() {}", "jev-latest");
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(
            body["state"],
            state(&pair(), "fn alpha() {}", "fn beta() {}")
        );
        assert_eq!(body["questions"], questions());
        assert_eq!(body.as_object().expect("a map").len(), 3);
    }

    fn write_lines(lines: &[String]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        for line in lines {
            writeln!(file, "{line}").expect("write");
        }
        file
    }

    #[test]
    fn a_span_is_read_back_from_disk() {
        let lines: Vec<String> = (1..=5).map(|n| format!("line {n}")).collect();
        let file = write_lines(&lines);
        assert_eq!(
            read_span(file.path(), 2, 3).expect("in range"),
            "line 2\nline 3"
        );
        assert_eq!(
            read_span(file.path(), 5, 5).expect("the last line"),
            "line 5"
        );
    }

    #[test]
    fn a_span_past_the_end_of_the_file_is_an_error() {
        // The file changed after the scan located the function.
        let file = write_lines(&["only".to_owned()]);
        let err = read_span(file.path(), 1, 2).expect_err("line 2 is gone");
        assert!(err.to_string().contains("changed"), "{err}");
        assert!(read_span(file.path(), 0, 1).is_err(), "lines are 1-indexed");
        assert!(read_span(file.path(), 2, 1).is_err(), "an empty range");
    }

    #[test]
    fn an_unreadable_file_is_an_error_naming_it() {
        let path = Path::new("/nonexistent/cargo-crap/triage.rs");
        let err = read_span(path, 1, 1).expect_err("no such file");
        assert!(
            err.to_string()
                .contains("/nonexistent/cargo-crap/triage.rs"),
            "{err}"
        );
    }

    #[test]
    fn crlf_line_endings_come_back_as_newlines() {
        // Normalised on purpose: a CRLF and an LF checkout of the same code
        // send the same source, and so share cached verdicts.
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(b"fn a() {\r\n    1\r\n}\r\n")
            .expect("write");
        assert_eq!(
            read_span(file.path(), 1, 3).expect("in range"),
            "fn a() {\n    1\n}"
        );
    }

    /// FNV-1a over the bytes: stable across processes and toolchains.
    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn the_similarity_score_is_pinned_to_the_question_set_version() {
        // The score is sent as `structural_similarity` but is not part of the
        // cache key. If this fails, the fingerprinting changed what the model
        // is shown: bump QUESTION_SET_VERSION, then update the pinned score.
        use crate::duplicates::compare::find_pairs;
        use crate::duplicates::extract::functions_in_source;
        let source = "
            fn a(v: &[i32]) -> i32 { let mut t = 0; for x in v { if *x > 0 { t += x; } } t }
            fn b(v: &[i32]) -> i32 { let mut t = 0; for x in v { if *x > 0 { t += x; } else { t -= 1; } } t }
        ";
        let functions = functions_in_source(source, Path::new("pin.rs")).expect("parses");
        let pairs = find_pairs(&functions, 0.0);
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (QUESTION_SET_VERSION, pairs[0].score.to_string()),
            (1, "0.46875".to_owned()),
            "the similarity score changed; bump QUESTION_SET_VERSION"
        );
    }

    #[test]
    fn the_question_set_is_pinned_to_its_version() {
        // The version is part of the cache key. If this fails, the questions
        // changed: bump QUESTION_SET_VERSION, then update both values here.
        assert_eq!(
            (
                QUESTION_SET_VERSION,
                fnv1a(questions().to_string().as_bytes())
            ),
            (1, 0x2d2d_1a84_5c08_974a),
            "the question set changed; bump QUESTION_SET_VERSION"
        );
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(read_span(Path::new("/nonexistent/cargo-crap/triage.rs"), 1, 1).is_err());
    }

    #[test]
    fn a_pair_is_built_from_the_files_on_disk() {
        let a = write_lines(&["fn alpha() {".into(), "    1".into(), "}".into()]);
        let b = write_lines(&["// header".into(), "fn beta() { 2 }".into()]);
        let pair = DuplicatePair {
            first: Location {
                file: a.path().to_path_buf(),
                start_line: 1,
                end_line: 3,
                name: "alpha".into(),
            },
            second: Location {
                file: b.path().to_path_buf(),
                start_line: 2,
                end_line: 2,
                name: "beta".into(),
            },
            score: 0.9,
        };
        let body = build(&pair, "jev-latest").expect("both spans read");
        assert_eq!(
            body["state"]["function_a"]["source"],
            "fn alpha() {\n    1\n}"
        );
        assert_eq!(body["state"]["function_b"]["source"], "fn beta() { 2 }");
        assert_eq!(body["model"], "jev-latest");
    }

    proptest! {
        /// The source sent for a side is exactly its lines — nothing before,
        /// nothing after, nothing in between changed.
        #[test]
        fn a_span_is_exactly_its_lines(
            lines in proptest::collection::vec("[^\r\n]{0,20}", 1..30),
            a in 0usize..30,
            b in 0usize..30,
        ) {
            let n = lines.len();
            let (start, end) = {
                let (x, y) = (a % n + 1, b % n + 1);
                if x <= y { (x, y) } else { (y, x) }
            };
            let file = write_lines(&lines);
            prop_assert_eq!(
                read_span(file.path(), start, end).expect("in range"),
                lines[start - 1..end].join("\n")
            );
        }
    }
}
