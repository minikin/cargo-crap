mod istanbul;

use crate::InputError;
use cargo_crap_core::coverage::{StrictLcovFile, read_strict_lcov};
use std::io;
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
pub(crate) enum CoverageData {
    Statements(StatementFiles),
    Lines(Vec<StrictLcovFile>),
}

#[derive(Debug, PartialEq)]
pub struct Coverage(pub(crate) CoverageData);

impl Coverage {
    #[must_use]
    pub fn unit(&self) -> CoverageUnit {
        match &self.0 {
            CoverageData::Statements(_) => CoverageUnit::Statements,
            CoverageData::Lines(_) => CoverageUnit::Lines,
        }
    }

    /// Keys retain their producer spellings and traversal order, including files with no observations.
    #[doc(hidden)]
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        match &self.0 {
            CoverageData::Statements(files) => files.iter().map(|(key, _)| key.as_str()).collect(),
            CoverageData::Lines(files) => files.iter().map(|file| file.key.as_str()).collect(),
        }
    }
}

fn js_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c != '\u{85}' && c.is_whitespace())
}

/// # Errors
/// Reading fails on filesystem errors, unknown formats, malformed JSON, invalid statement maps or invalid LCOV records.
pub fn read_coverage(
    cwd: &Path,
    path: &Path,
) -> Result<Coverage, InputError> {
    read_with(cwd, path, |label| std::fs::read(label))
}

fn read_error(
    label: &Path,
    error: io::Error,
) -> InputError {
    let code = match error.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::PermissionDenied if label.is_dir() => "EISDIR",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        _ => return InputError::at(label, error),
    };
    InputError::at(label, code)
}

fn read_with(
    cwd: &Path,
    path: &Path,
    read: impl FnOnce(&Path) -> io::Result<Vec<u8>>,
) -> Result<Coverage, InputError> {
    let base = std::path::absolute(cwd).map_err(|error| read_error(cwd, error))?;
    let label = crate::discover::resolve(&base, path);
    let bytes = read(&label).map_err(|error| read_error(&label, error))?;
    let decoded = String::from_utf8_lossy(&bytes);
    let text = decoded.strip_prefix('\u{feff}').unwrap_or(&decoded);
    let trimmed = text.trim_start_matches(js_whitespace);
    if trimmed.starts_with('{') {
        return istanbul::read(trimmed, &label)
            .map(|files| Coverage(CoverageData::Statements(files)));
    }
    if text.split('\n').any(|line| line.starts_with("SF:")) {
        return read_strict_lcov(text, &label)
            .map(|files| Coverage(CoverageData::Lines(files)))
            .map_err(|error| InputError::reported(&label, error.to_string()));
    }
    Err(InputError::at(&label, "unrecognized coverage format"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denied_read_identifies_the_resolved_report() {
        let cwd = tempfile::tempdir().unwrap();
        let expected = cwd.path().join("denied");
        let error = read_with(cwd.path(), Path::new("reports/../denied"), |label| {
            assert_eq!(label, expected);
            Err(std::io::ErrorKind::PermissionDenied.into())
        })
        .unwrap_err();
        assert_eq!(error.path(), expected);
        assert_eq!(error.to_string(), format!("{}: EACCES", expected.display()));
    }

    #[test]
    fn filesystem_errors_distinguish_a_directory_from_a_nondirectory_parent() {
        let cwd = tempfile::tempdir().unwrap();
        let directory = cwd.path().join("reports");
        std::fs::create_dir(&directory).unwrap();
        for (label, kind, expected) in [
            (&directory, io::ErrorKind::PermissionDenied, "EISDIR"),
            (
                &directory.join("file/child"),
                io::ErrorKind::NotADirectory,
                "ENOTDIR",
            ),
        ] {
            let error = read_with(cwd.path(), label, |_| Err(kind.into())).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("{}: {expected}", label.display())
            );
        }
    }

    #[test]
    fn json_detection_accepts_the_full_leading_whitespace_set() {
        let whitespace = "\t\n\u{b}\u{c}\r \u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";
        let cwd = tempfile::tempdir().unwrap();
        let label = cwd.path().join("cov");
        for ch in whitespace.chars() {
            std::fs::write(&label, format!("{ch}{{}}")).unwrap();
            assert_eq!(
                read_coverage(cwd.path(), &label).unwrap().unit(),
                CoverageUnit::Statements,
                "{ch:?}"
            );
        }
    }

    #[test]
    fn lcov_dispatch_preserves_the_core_readers_counts_and_diagnostics() {
        let cwd = tempfile::tempdir().unwrap();
        let path = cwd.path().join("cov");
        std::fs::write(&path, "TN:\nSF:./a\nDA:2,0\nSF:b\nSF:./a\nDA:2,7").unwrap();
        let report = read_coverage(cwd.path(), &path).unwrap();
        let CoverageData::Lines(files) = report.0 else {
            panic!("expected lines")
        };
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].key, "./a");
        assert_eq!(files[0].lines[&2], 7.0);
        assert_eq!(files[1].lines.len(), 0);
    }

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
