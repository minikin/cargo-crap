mod istanbul;

use crate::InputError;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageUnit {
    Statements,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub(crate) struct Point<C = f64> {
    pub line: f64,
    pub column: C,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Statement {
    pub start: Point,
    pub end: Point<Option<f64>>,
    pub hits: f64,
}

type StatementFiles = Vec<(String, Vec<Statement>)>;

#[derive(Debug, PartialEq)]
enum CoverageData {
    Statements(StatementFiles),
}

#[derive(Debug, PartialEq)]
pub struct Coverage(CoverageData);

impl Coverage {
    #[must_use]
    pub fn unit(&self) -> CoverageUnit {
        match &self.0 {
            CoverageData::Statements(_) => CoverageUnit::Statements,
        }
    }

    /// Keys retain their producer spellings and traversal order, including files with no observations.
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        match &self.0 {
            CoverageData::Statements(files) => files.iter().map(|(key, _)| key.as_str()).collect(),
        }
    }
}

fn js_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c != '\u{85}' && c.is_whitespace())
}

/// # Errors
/// The diagnostic identifies the report and the first invalid coverage entry.
pub fn read_coverage(
    cwd: &Path,
    path: &Path,
) -> Result<Coverage, InputError> {
    let label = cwd.join(path);
    let bytes = std::fs::read(&label).map_err(|err| InputError::at(&label, err))?;
    let decoded = String::from_utf8_lossy(&bytes);
    let text = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);
    let trimmed = text.trim_start_matches(js_whitespace);
    if trimmed.starts_with('{') {
        return istanbul::read(trimmed, &label)
            .map(|files| Coverage(CoverageData::Statements(files)));
    }
    Err(InputError::at(&label, "unrecognized coverage format"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_detection_accepts_ecmascript_leading_whitespace() {
        let cwd = tempfile::tempdir().unwrap();
        let path = cwd.path().join("report");
        for prefix in ["\u{feff} \t\n", " \u{feff}", "\u{1680}\u{2029}"] {
            std::fs::write(&path, format!("{prefix}{{}}")).unwrap();
            let data = read_coverage(cwd.path(), Path::new("report")).unwrap();
            assert_eq!(data.unit(), CoverageUnit::Statements);
            assert_eq!(data.keys(), Vec::<&str>::new());
        }
        for prefix in ["\u{85}", "a"] {
            std::fs::write(&path, format!("{prefix}{{}}")).unwrap();
            assert!(
                read_coverage(cwd.path(), &path)
                    .unwrap_err()
                    .to_string()
                    .ends_with("unrecognized coverage format")
            );
        }
    }
}
