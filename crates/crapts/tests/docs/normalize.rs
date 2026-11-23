const VERSION_PATHS: [&[&str]; 5] = [
    &["version"],
    &["regression_gate", "version"],
    &["regression_gate", "baseline_version"],
    &["changed", "version"],
    &["runs", "[]", "tool", "driver", "version"],
];
const PATH_KEYS: [&str; 3] = ["file", "previous_file", "uri"];

pub struct Normalizer<'a> {
    pub windows: bool,
    pub version: &'a str,
    pub fixture_paths: &'a [&'a str],
}

impl Normalizer<'_> {
    pub fn apply(
        &self,
        stream: &str,
        json: bool,
    ) -> String {
        if json {
            self.json(stream)
        } else {
            self.text(stream)
        }
    }

    fn text(
        &self,
        stream: &str,
    ) -> String {
        let own = format!("crapts {}", self.version);
        let mut out: String = stream
            .split_inclusive('\n')
            .map(|line| {
                if line.trim_end_matches(['\n', '\r']) == own {
                    line.replacen(self.version, "<version>", 1)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        if self.windows {
            for path in self.fixture_paths {
                out = replace_path(&out, &path.replace('/', "\\"), path);
            }
        }
        out
    }

    fn json(
        &self,
        stream: &str,
    ) -> String {
        let mut out = String::with_capacity(stream.len());
        let mut copied = 0;
        for value in string_values(stream) {
            let raw = &stream[value.start..value.end];
            let keys: Vec<&str> = value.path.iter().map(String::as_str).collect();
            let replaced = if raw == self.version && VERSION_PATHS.contains(&keys.as_slice()) {
                "<version>".to_owned()
            } else if self.windows && keys.last().is_some_and(|k| PATH_KEYS.contains(k)) {
                raw.replace("\\\\", "/")
            } else {
                continue;
            };
            out.push_str(&stream[copied..value.start]);
            out.push_str(&replaced);
            copied = value.end;
        }
        out.push_str(&stream[copied..]);
        out
    }
}

fn path_char(c: char) -> bool {
    c.is_alphanumeric() || "_-\\/".contains(c)
}

/// Replaces `native` only where it stands as a whole path, so a longer
/// name that merely contains it keeps its bytes.
fn replace_path(
    text: &str,
    native: &str,
    portable: &str,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (at, _) in text.match_indices(native) {
        let before = text[..at].chars().next_back();
        let rest = &text[at + native.len()..];
        let mut after = rest.chars();
        let next = after.next();
        let continues = next.is_some_and(path_char)
            || (next == Some('.') && after.next().is_some_and(char::is_alphanumeric));
        if before.is_some_and(|c| path_char(c) || c == '.') || continues {
            continue;
        }
        out.push_str(&text[copied..at]);
        out.push_str(portable);
        copied = at + native.len();
    }
    out.push_str(&text[copied..]);
    out
}

pub struct StringValue {
    pub path: Vec<String>,
    pub start: usize,
    pub end: usize,
}

enum Frame {
    Object {
        key: Option<String>,
        awaiting_key: bool,
    },
    Array,
}

/// Values are spliced by byte range, never re-serialized, so every byte
/// outside a replaced value stays as captured. `[]` stands for an array element.
pub fn string_values(json: &str) -> Vec<StringValue> {
    let bytes = json.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => stack.push(Frame::Object {
                key: None,
                awaiting_key: true,
            }),
            b'[' => stack.push(Frame::Array),
            b'}' | b']' => {
                stack.pop();
            },
            b',' => {
                if let Some(Frame::Object { awaiting_key, .. }) = stack.last_mut() {
                    *awaiting_key = true;
                }
            },
            b'"' => {
                let end = string_end(bytes, i + 1);
                let raw = &json[i + 1..end];
                if let Some(Frame::Object {
                    key,
                    awaiting_key: awaiting @ true,
                }) = stack.last_mut()
                {
                    *key = Some(raw.to_owned());
                    *awaiting = false;
                } else {
                    found.push(StringValue {
                        path: path_of(&stack),
                        start: i + 1,
                        end,
                    });
                }
                i = end;
            },
            _ => {},
        }
        i += 1;
    }
    found
}

fn string_end(
    bytes: &[u8],
    mut i: usize,
) -> usize {
    while i < bytes.len() && bytes[i] != b'"' {
        i += if bytes[i] == b'\\' { 2 } else { 1 };
    }
    i.min(bytes.len())
}

fn path_of(stack: &[Frame]) -> Vec<String> {
    stack
        .iter()
        .map(|frame| match frame {
            Frame::Object { key, .. } => key.clone().unwrap_or_default(),
            Frame::Array => "[]".to_owned(),
        })
        .collect()
}
