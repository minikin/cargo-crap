//! Optional GitHub source-link wrapping for markdown / pr-comment output.
//!
//! When the user (or CI) provides a repo URL and a commit ref, Function and
//! Location cells are wrapped in `[`text`](url#Lline)` markdown links.
//! Otherwise the cells render as plain code spans — the renderers themselves
//! never have to branch on link presence beyond passing `Option<&SourceLinks>`
//! through to [`linkify`].

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::path::{Path, PathBuf};

/// RFC 3986 unreserved bytes plus `/`, which must remain a path separator.
const PATH_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// Repo URL + commit ref used by `markdown` / `pr-comment` renderers to wrap
/// Function and Location cells in clickable source links.
///
/// Constructed once at the CLI layer (or by library callers) and threaded
/// through as `Option<&SourceLinks>` — `None` means "no flags set, render
/// plain code spans like before".
#[derive(Clone, Debug)]
pub struct SourceLinks {
    repo_url: String,
    commit_ref: String,
    /// The repository root on disk, which absolute paths are made relative
    /// to. `None` uses the process's working directory.
    root: Option<PathBuf>,
    /// The workspace root a workspace-mode report writes its paths below
    /// (spec 41). A relative path is joined to it before `root` is
    /// stripped, so the URL is the one the absolute path would give.
    workspace_root: Option<PathBuf>,
}

impl SourceLinks {
    /// Trims a trailing `/` off `repo_url` so URL composition produces exactly
    /// one slash between the base and `/blob/...`.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "callers always have fresh Strings (CLI flags / env vars); commit_ref is moved into self; taking `&str` would force `.to_string()` at every call site"
    )]
    #[must_use]
    pub fn new(
        repo_url: String,
        commit_ref: String,
    ) -> Self {
        Self {
            repo_url: repo_url.trim_end_matches('/').to_string(),
            commit_ref,
            root: None,
            workspace_root: None,
        }
    }

    /// The same links, with absolute paths made relative to `root` rather
    /// than the process's working directory. The CLI passes the working
    /// directory it resolved once, so links and displayed locations agree.
    #[must_use]
    pub fn with_root(
        mut self,
        root: PathBuf,
    ) -> Self {
        self.root = Some(root);
        self
    }

    /// The same links, for a report whose paths are written below
    /// `workspace_root` (spec 41).
    #[must_use]
    pub fn with_workspace_root(
        mut self,
        workspace_root: PathBuf,
    ) -> Self {
        self.workspace_root = Some(workspace_root);
        self
    }

    /// Build a deep link to `file` at `line` on the configured ref.
    ///
    /// GitHub URLs require forward slashes regardless of host OS. On
    /// Windows `Path::display()` emits `src\foo.rs`, which would land in
    /// the URL verbatim and 404 on github.com — so we normalize backslashes
    /// to forward slashes before composing the URL.
    #[must_use]
    pub fn url_for(
        &self,
        file: &Path,
        line: usize,
    ) -> String {
        let path = file.to_string_lossy().replace('\\', "/");
        let path = utf8_percent_encode(&path, PATH_ENCODE_SET);
        format!(
            "{}/blob/{}/{}#L{}",
            self.repo_url, self.commit_ref, path, line
        )
    }
}

/// Decide what path to embed into a `/blob/<ref>/...` URL for `path`.
///
/// The URL prefix is **always the repo root (== CWD)**, never the LCP used
/// for the visible Location text. If we shared the LCP, a rendered set that
/// happens to live entirely under `src/` would strip `src/` from the URL
/// too, yielding `host/repo/blob/<sha>/main.rs` which 404s.
///
/// Returns the repo-relative form when `path` is already relative (cargo
/// crap reports relative paths when not invoked with `--workspace`) or
/// absolute under `root` (the process's working directory when `None`).
/// Returns `None` otherwise — those rows fall back to
/// plain code spans rather than emit a broken
/// `host/repo/blob/<sha>//abs/...` URL.
fn link_path(
    path: &Path,
    root: Option<&Path>,
) -> Option<PathBuf> {
    if path.is_relative() {
        return Some(path.to_path_buf());
    }
    let cwd = root_or_cwd(root)?;
    path.strip_prefix(&cwd)
        .ok()
        .map(std::path::Path::to_path_buf)
}

/// `root` when given, else the process's working directory, or `None` when
/// that cannot be read.
fn root_or_cwd(root: Option<&Path>) -> Option<PathBuf> {
    root.map_or_else(
        || std::env::current_dir().ok(),
        |root| Some(root.to_path_buf()),
    )
}

/// Wrap `inner` (already-formatted, e.g. with backticks) in a markdown link
/// iff both `links` and a usable (repo-relative) URL path are available;
/// otherwise return `inner` unchanged.
pub(crate) fn linkify(
    inner: String,
    links: Option<&SourceLinks>,
    file: &Path,
    line: usize,
) -> String {
    let Some(links) = links else {
        return inner;
    };
    // `join` keeps an absolute `file` as it is.
    let file = links.workspace_root.as_deref().map_or_else(
        || file.to_path_buf(),
        |workspace_root| workspace_root.join(file),
    );
    match link_path(&file, links.root.as_deref()) {
        Some(path) => format!("[{inner}]({})", links.url_for(&path, line)),
        None => inner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_relative_path_links_as_its_absolute_path_would() {
        // A root without a drive letter is not absolute on Windows.
        let ws = PathBuf::from(if cfg!(windows) { r"C:\ws" } else { "/ws" });
        let links = SourceLinks::new("https://github.com/o/r".into(), "abc".into())
            .with_root(ws.join("crates/beta"))
            .with_workspace_root(ws.clone());
        let at = |file: &Path| linkify("x".into(), Some(&links), file, 3);
        let relative = Path::new("crates/beta/src/a.rs");
        assert_eq!(
            at(relative),
            at(&ws.join(relative)),
            "the relative and the absolute path link alike"
        );
        assert_eq!(
            at(relative),
            "[x](https://github.com/o/r/blob/abc/src/a.rs#L3)"
        );
        assert_eq!(
            at(Path::new("crates/alpha/src/a.rs")),
            "x",
            "outside the root: no link"
        );
    }

    #[test]
    fn source_links_url_for_joins_components_with_one_slash() {
        let l = SourceLinks::new("https://github.com/owner/repo".into(), "abc123".into());
        let url = l.url_for(Path::new("src/foo.rs"), 42);
        assert_eq!(
            url,
            "https://github.com/owner/repo/blob/abc123/src/foo.rs#L42"
        );
    }

    #[test]
    fn source_links_strips_trailing_slash_from_repo_url() {
        let l = SourceLinks::new("https://github.com/owner/repo/".into(), "abc123".into());
        let url = l.url_for(Path::new("src/foo.rs"), 1);
        assert!(
            !url.contains("repo//blob"),
            "trailing slash must be normalized: {url}"
        );
        assert!(url.contains("/repo/blob/abc123/"));
    }

    #[test]
    fn source_links_url_uses_forward_slashes_even_for_windows_input() {
        // GitHub URLs always use `/`. A Windows-style backslash path must
        // be normalized before it lands in the URL, otherwise links break
        // on github.com regardless of which OS rendered them.
        let l = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let url = l.url_for(Path::new(r"src\foo.rs"), 1);
        assert!(
            !url.contains('\\'),
            "URL must contain no backslashes, got: {url}"
        );
        assert_eq!(url, "https://github.com/o/r/blob/sha/src/foo.rs#L1");
    }

    #[test]
    fn source_links_percent_encodes_reserved_path_bytes() {
        let l = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let url = l.url_for(Path::new("src/a file#50%?(draft).rs"), 7);
        assert_eq!(
            url,
            "https://github.com/o/r/blob/sha/src/a%20file%2350%25%3F%28draft%29.rs#L7"
        );
    }

    #[test]
    fn source_links_percent_encodes_utf8_path_bytes() {
        let l = SourceLinks::new("https://github.com/o/r".into(), "sha".into());
        let url = l.url_for(Path::new("src/数据.rs"), 9);
        assert_eq!(
            url,
            "https://github.com/o/r/blob/sha/src/%E6%95%B0%E6%8D%AE.rs#L9"
        );
    }

    #[test]
    fn an_explicit_root_decides_which_paths_link() {
        // Absolute on every platform: `/work` has no drive on Windows.
        let root = std::env::temp_dir().join("work");
        let links =
            SourceLinks::new("https://github.com/o/r".into(), "abc".into()).with_root(root.clone());
        let inside = root.join("src").join("a.rs");
        assert_eq!(
            linkify("`f`".into(), Some(&links), &inside, 3),
            "[`f`](https://github.com/o/r/blob/abc/src/a.rs#L3)"
        );
        let outside = std::env::temp_dir().join("elsewhere").join("a.rs");
        assert_eq!(
            linkify("`f`".into(), Some(&links), &outside, 3),
            "`f`",
            "outside the root stays plain"
        );
    }
}
