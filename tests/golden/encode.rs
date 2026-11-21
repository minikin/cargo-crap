//! One case's run as one text document that insta stores without loss.
//!
//! insta trims trailing whitespace and turns CRLF into LF before it
//! compares, so raw output would lose a final newline or a `\r` unnoticed.
//! The document escapes every byte that could be lost or hidden, closes each
//! stream with `<<end>>` right after its last byte, and puts the exit status
//! last, so no stream sits at the trimmed end. It opens with the case's ID:
//! two cases can print the same bytes (`w_human` and `w_warm_cache` do), and
//! only the ID tells a run compared with the wrong snapshot from the right one.

/// What a case's process left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Exit(i32),
    Signal(i32),
}

const END: &str = "<<end>>";

pub fn encode(
    case: &str,
    run: &Run,
) -> String {
    let status = match run.status {
        Status::Exit(code) => format!("exit: {code}"),
        Status::Signal(signal) => format!("signal: {signal}"),
    };
    format!(
        "case: {case}\nstdout:\n{}{END}\nstderr:\n{}{END}\n{status}",
        escape(&run.stdout),
        escape(&run.stderr)
    )
}

pub fn matches(
    case: &str,
    recorded: &str,
    run: &Run,
) -> bool {
    recorded == encode(case, run)
}

pub fn decode(document: &str) -> Option<(String, Run)> {
    let (case, rest) = document.strip_prefix("case: ")?.split_once('\n')?;
    let rest = rest.strip_prefix("stdout:\n")?;
    let (stdout, rest) = unescape_stream(rest)?;
    let rest = rest.strip_prefix("\nstderr:\n")?;
    let (stderr, rest) = unescape_stream(rest)?;
    let rest = rest.strip_prefix('\n')?;
    let status = if let Some(code) = rest.strip_prefix("exit: ") {
        Status::Exit(code.parse().ok()?)
    } else {
        Status::Signal(rest.strip_prefix("signal: ")?.parse().ok()?)
    };
    let run = Run {
        stdout,
        stderr,
        status,
    };
    Some((case.to_owned(), run))
}

fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        let valid = chunk.valid();
        for (at, c) in valid.char_indices() {
            escape_char(c, valid[at..].starts_with(END), &mut out);
        }
        for byte in chunk.invalid() {
            push_hex(*byte, &mut out);
        }
    }
    out
}

fn escape_char(
    c: char,
    opens_end_marker: bool,
    out: &mut String,
) {
    match c {
        '\\' => out.push_str("\\\\"),
        '\r' => out.push_str("\\r"),
        '\x1b' => out.push_str("\\e"),
        '<' if opens_end_marker => out.push_str("\\<"),
        '\n' | '\t' => out.push(c),
        c if c.is_control() => {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                push_hex(byte, out);
            }
        },
        c => out.push(c),
    }
}

fn push_hex(
    byte: u8,
    out: &mut String,
) {
    use std::fmt::Write as _;
    write!(out, "\\x{byte:02x}").expect("writing to a String");
}

fn unescape_stream(text: &str) -> Option<(Vec<u8>, &str)> {
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some(after) = rest.strip_prefix(END) {
            return Some((out, after));
        }
        if let Some(escaped) = rest.strip_prefix('\\') {
            let (byte, used) = unescape_one(escaped)?;
            out.push(byte);
            at += 1 + used;
        } else {
            let c = rest.chars().next()?;
            out.extend_from_slice(&rest.as_bytes()[..c.len_utf8()]);
            at += c.len_utf8();
        }
    }
    None
}

fn unescape_one(escaped: &str) -> Option<(u8, usize)> {
    match escaped.as_bytes().first()? {
        b'\\' => Some((b'\\', 1)),
        b'r' => Some((b'\r', 1)),
        b'e' => Some((0x1b, 1)),
        b'<' => Some((b'<', 1)),
        b'x' => {
            let hex = escaped.get(1..3)?;
            Some((u8::from_str_radix(hex, 16).ok()?, 3))
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn run(
        stdout: &[u8],
        stderr: &[u8],
        status: Status,
    ) -> Run {
        Run {
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            status,
        }
    }

    /// What insta does to a file snapshot before comparing it.
    fn insta_normalised(text: &str) -> String {
        text.trim_end().replace("\r\n", "\n")
    }

    #[test]
    fn a_document_names_its_case_both_streams_and_ends_with_the_status() {
        let doc = encode("p_human", &run(b"table\n", b"warning: x", Status::Exit(1)));
        assert_eq!(
            doc,
            "case: p_human\nstdout:\ntable\n<<end>>\nstderr:\nwarning: x<<end>>\nexit: 1"
        );
    }

    #[test]
    fn lossy_bytes_are_escaped_visibly() {
        let doc = encode(
            "c",
            &run(b"a\r\nb\\c\x1b[1m\x07\xff\t", b"", Status::Signal(9)),
        );
        assert_eq!(
            doc,
            "case: c\nstdout:\na\\r\nb\\\\c\\e[1m\\x07\\xff\t<<end>>\nstderr:\n<<end>>\nsignal: 9"
        );
    }

    #[test]
    fn table_glyphs_round_trip() {
        let glyphs = run("│ ▲ ┆ ░░░░ ö ✓ ✗\n".as_bytes(), b"", Status::Exit(0));
        assert_eq!(
            decode(&encode("c", &glyphs)),
            Some(("c".to_owned(), glyphs))
        );
    }

    #[test]
    fn the_end_marker_inside_a_stream_is_escaped() {
        let marked = run(b"x<<end>>y", b"", Status::Exit(0));
        let doc = encode("c", &marked);
        assert!(
            doc.starts_with("case: c\nstdout:\nx\\<<end>>y<<end>>\n"),
            "{doc}"
        );
        assert_eq!(decode(&doc), Some(("c".to_owned(), marked)));
    }

    #[test]
    fn a_malformed_document_does_not_decode() {
        for doc in [
            "",
            "stdout:\n<<end>>\nstderr:\n<<end>>\nexit: 0",
            "case: c\nstdout:\nno end marker",
            "case: c\nstdout:\n<<end>>\nstderr:\n<<end>>\nexit: x",
            "case: c\nstdout:\n<<end>>\nstderr:\n<<end>>\nsignal: x",
            "case: c\nstdout:\n<<end>>\nstderr:\n<<end>>\nstatus: 0",
            "case: c\nstdout:\n\\q<<end>>\nstderr:\n<<end>>\nexit: 0",
        ] {
            assert_eq!(decode(doc), None, "{doc:?}");
        }
    }

    #[test]
    fn a_dropped_stderr_changes_the_encoding() {
        let recorded = run(b"report\n", b"warning: scope mismatch\n", Status::Exit(0));
        let dropped = run(b"report\n", b"", Status::Exit(0));
        assert!(!matches("c", &encode("c", &recorded), &dropped));
    }

    #[test]
    fn a_run_matches_only_its_own_cases_snapshot() {
        let same = run(b"identical\n", b"", Status::Exit(0));
        let recorded = encode("w_human", &same);
        assert!(matches("w_human", &recorded, &same));
        assert!(!matches("w_warm_cache", &recorded, &same));
    }

    #[test]
    fn another_signal_changes_the_encoding() {
        let term = run(b"", b"", Status::Signal(15));
        let kill = run(b"", b"", Status::Signal(9));
        assert!(!matches("c", &encode("c", &term), &kill));
        let exit = run(b"", b"", Status::Exit(9));
        assert!(!matches("c", &encode("c", &exit), &kill));
    }

    fn any_stream() -> impl Strategy<Value = Vec<u8>> {
        let bytes = proptest::collection::vec(
            prop_oneof![
                any::<u8>(),
                Just(b'\r'),
                Just(b'\n'),
                Just(b'\\'),
                Just(b'<'),
                Just(b'>'),
                Just(b'e'),
            ],
            0..64,
        );
        prop_oneof![bytes, any::<String>().prop_map(String::into_bytes)]
    }

    fn any_status() -> impl Strategy<Value = Status> {
        prop_oneof![
            (-3i32..300).prop_map(Status::Exit),
            (1i32..32).prop_map(Status::Signal),
        ]
    }

    fn any_run() -> impl Strategy<Value = Run> {
        (any_stream(), any_stream(), any_status()).prop_map(|(stdout, stderr, status)| Run {
            stdout,
            stderr,
            status,
        })
    }

    /// A one-byte edit of a run: one stream's byte changed, added or
    /// removed, or the status moved.
    fn edited(
        original: &Run,
        which: u8,
        at: usize,
        byte: u8,
    ) -> Run {
        let mut edit = original.clone();
        let stream = if which.is_multiple_of(2) {
            &mut edit.stdout
        } else {
            &mut edit.stderr
        };
        // The low bit picks the stream, the next two the edit, so every
        // edit reaches both streams.
        match (which >> 1) % 4 {
            0 if !stream.is_empty() => {
                let i = at % stream.len();
                stream[i] = stream[i].wrapping_add(byte.max(1));
            },
            1 => stream.push(byte),
            2 if !stream.is_empty() => {
                stream.remove(at % stream.len());
            },
            _ => {
                edit.status = match edit.status {
                    Status::Exit(code) => Status::Exit(code.wrapping_add(1)),
                    Status::Signal(signal) => Status::Signal(signal.wrapping_add(1)),
                }
            },
        }
        edit
    }

    proptest! {
        #[test]
        fn the_snapshot_encoding_is_exact_and_reversible(r in any_run(), case in "[a-z_0-9]{1,40}") {
            prop_assert_eq!(decode(&encode(&case, &r)), Some((case, r)));
        }

        #[test]
        fn insta_normalisation_leaves_an_encoding_unchanged(r in any_run()) {
            let doc = encode("c", &r);
            prop_assert_eq!(insta_normalised(&doc), doc);
        }

        #[test]
        fn a_changed_stream_exit_status_or_snapshot_pairing_fails_its_case(
            r in any_run(),
            which in any::<u8>(),
            at in any::<usize>(),
            byte in any::<u8>(),
        ) {
            let edit = edited(&r, which, at, byte);
            prop_assume!(edit != r);
            let recorded = encode("c", &r);
            prop_assert!(matches("c", &recorded, &r));
            prop_assert!(!matches("c", &recorded, &edit));
        }

        #[test]
        fn another_cases_snapshot_fails_the_comparison(
            r in any_run(),
            mine in "[a-z_]{1,20}",
            theirs in "[a-z_]{1,20}",
        ) {
            prop_assume!(mine != theirs);
            prop_assert!(!matches(&mine, &encode(&theirs, &r), &r));
        }
    }
}
