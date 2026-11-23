use std::fmt;

pub const PREFIX: &str = "<!-- crapts-";
const CLOSE: &str = "-->";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    Run {
        stream: Stream,
        exit: u8,
        args: Vec<String>,
    },
    Help,
}

impl Marker {
    pub fn args(&self) -> Vec<String> {
        match self {
            Marker::Run { args, .. } => args.clone(),
            Marker::Help => vec!["--help".to_owned()],
        }
    }

    pub fn exit(&self) -> u8 {
        match self {
            Marker::Run { exit, .. } => *exit,
            Marker::Help => 0,
        }
    }

    pub fn stream(&self) -> Stream {
        match self {
            Marker::Run { stream, .. } => *stream,
            Marker::Help => Stream::Stdout,
        }
    }

    pub fn effective_format(&self) -> Option<String> {
        let Marker::Run { args, .. } = self else {
            return None;
        };
        let informational = ["--help", "-h", "--version", "-V", "--print-config"];
        if args.iter().any(|a| informational.contains(&a.as_str())) {
            return None;
        }
        let format = args
            .iter()
            .enumerate()
            .find_map(|(i, a)| match a.strip_prefix("--format") {
                Some("") => args.get(i + 1).cloned(),
                Some(rest) => rest.strip_prefix('=').map(str::to_owned),
                None => None,
            })
            .unwrap_or_else(|| "text".to_owned());
        Some(if format == "human" {
            "text".to_owned()
        } else {
            format
        })
    }
}

impl fmt::Display for Marker {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        let Marker::Run { stream, exit, args } = self else {
            return f.write_str("<!-- crapts-help -->");
        };
        let kind = match stream {
            Stream::Stdout => "output",
            Stream::Stderr => "stderr",
        };
        let exit = match (stream, exit) {
            (Stream::Stdout, 0) => String::new(),
            _ => format!(" exit={exit}"),
        };
        let words: Vec<String> = args.iter().map(|a| quote(a)).collect();
        write!(f, "<!-- crapts-{kind}{exit}: {} -->", words.join(" "))
    }
}

fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && !word
            .chars()
            .any(|c| c.is_whitespace() || "'\"|;>&#".contains(c));
    if plain {
        word.to_owned()
    } else if word.contains('"') {
        format!("'{word}'")
    } else {
        format!("\"{word}\"")
    }
}

pub fn split(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.get_or_insert_default().push(c);
            }
            continue;
        }
        let ends = matches!(c, '|' | ';' | '>')
            || (c == '&' && chars.peek() == Some(&'&'))
            || (c == '#' && word.is_none());
        if ends {
            break;
        }
        if c.is_whitespace() {
            words.extend(word.take());
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            word.get_or_insert_default();
        } else {
            word.get_or_insert_default().push(c);
        }
    }
    if let Some(q) = quote {
        return Err(format!("an unterminated quote ({q})"));
    }
    words.extend(word);
    Ok(words)
}

pub fn parse(line: &str) -> Result<Marker, String> {
    let body = line
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(CLOSE))
        .ok_or_else(|| "a marker that is not one `<!-- crapts-... -->` line".to_owned())?;
    let (head, args) = match body.split_once(':') {
        Some((head, args)) => (head, Some(args)),
        None => (body, None),
    };
    let mut head = head.split_whitespace();
    let kind = head.next().unwrap_or_default();
    let exit = head.next().map(parse_exit).transpose()?;
    if let Some(extra) = head.next() {
        return Err(format!("an unexpected word `{extra}` before the colon"));
    }
    let args = args.map(split).transpose()?;
    build(kind, exit, args)
}

fn parse_exit(word: &str) -> Result<u8, String> {
    match word.strip_prefix("exit=") {
        Some("0") => Ok(0),
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        Some(code) => Err(format!("an exit code that is not 0, 1 or 2: `{code}`")),
        None => Err(format!("an unexpected word `{word}` where exit=N belongs")),
    }
}

fn build(
    kind: &str,
    exit: Option<u8>,
    args: Option<Vec<String>>,
) -> Result<Marker, String> {
    let stream = match kind {
        "help" if exit.is_some() => return Err("a help marker with an exit code".to_owned()),
        "help" if args.is_some() => return Err("a help marker with arguments".to_owned()),
        "help" => return Ok(Marker::Help),
        "output" => Stream::Stdout,
        "stderr" if exit.is_none() => return Err("a stderr marker without exit=".to_owned()),
        "stderr" => Stream::Stderr,
        other => return Err(format!("an unknown marker kind `crapts-{other}`")),
    };
    let args = args.unwrap_or_default();
    if args.is_empty() {
        return Err(format!("a crapts-{kind} marker without arguments"));
    }
    Ok(Marker::Run {
        stream,
        exit: exit.unwrap_or(0),
        args,
    })
}
