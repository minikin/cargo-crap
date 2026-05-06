//! Optional GitHub source-link wrapping for markdown / pr-comment output.
//!
//! When the user (or CI) provides a repo URL and a commit ref, Function and
//! Location cells are wrapped in `[`text`](url#Lline)` markdown links.
//! Otherwise the cells render as plain code spans — the renderers themselves
//! never have to branch on link presence beyond passing `Option<&SourceLinks>`
//! through to [`linkify`].

use std::path::{Path, PathBuf};

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
}

impl SourceLinks {
    /// Trims a trailing `/` off `repo_url` so URL composition produces exactly
    /// one slash between the base and `/blob/...`.
    pub fn new(
        repo_url: String,
        commit_ref: String,
    ) -> Self {
        Self {
            repo_url: repo_url.trim_end_matches('/').to_string(),
            commit_ref,
        }
    }

    /// Build a deep link to `file` at `line` on the configured ref.
    ///
    /// GitHub URLs require forward slashes regardless of host OS. On
    /// Windows `Path::display()` emits `src\foo.rs`, which would land in
    /// the URL verbatim and 404 on github.com — so we normalize backslashes
    /// to forward slashes before composing the URL.
    pub fn url_for(
        &self,
        file: &Path,
        line: usize,
    ) -> String {
        let path = file.to_string_lossy().replace('\\', "/");
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
/// absolute under CWD. Returns `None` otherwise — those rows fall back to
/// plain code spans rather than emit a broken
/// `host/repo/blob/<sha>//abs/...` URL.
fn link_path(path: &Path) -> Option<PathBuf> {
    if path.is_relative() {
        return Some(path.to_path_buf());
    }
    let cwd = std::env::current_dir().ok()?;
    path.strip_prefix(&cwd).ok().map(|p| p.to_path_buf())
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
    match (links, link_path(file)) {
        (Some(l), Some(p)) => format!("[{inner}]({})", l.url_for(&p, line)),
        _ => inner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
