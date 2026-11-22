use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use crate::InputError;

const DEFAULT_EXCLUDES: [&str; 10] = [
    "**/node_modules/**",
    "**/dist/**",
    "**/build/**",
    "**/coverage/**",
    "**/*.test.*",
    "**/*.spec.*",
    "**/*.stories.*",
    "**/*.min.js",
    "**/__tests__/**",
    "**/__mocks__/**",
];

#[derive(Debug)]
pub struct DiscoveryOptions {
    pub cwd: PathBuf,
    pub defaults: Vec<ExcludePattern>,
    pub excludes: Vec<ExcludePattern>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcludePattern {
    pub pattern: String,
    pub origin: GlobOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobOrigin {
    Default,
    Config(PathBuf),
    Flag,
}

impl DiscoveryOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let defaults = DEFAULT_EXCLUDES
            .iter()
            .map(|&pattern| ExcludePattern {
                pattern: pattern.to_owned(),
                origin: GlobOrigin::Default,
            })
            .collect();
        Self {
            cwd: cwd.into(),
            defaults,
            excludes: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct Discovery {
    pub files: Vec<PathBuf>,
    pub scope: DiscoveryScope,
    pub unused_excludes: Vec<ExcludePattern>,
}

#[derive(Debug)]
pub struct DiscoveryScope {
    cwd: PathBuf,
    invoked: PathBuf,
    repository: Option<PathBuf>,
    directories: Vec<PathBuf>,
    leaves: BTreeSet<PathBuf>,
    selected: BTreeSet<OsString>,
    defaults: Exclusions,
    extras: Exclusions,
}

impl DiscoveryScope {
    pub fn keeps(
        &self,
        path: &Path,
    ) -> Result<bool, InputError> {
        let absolute = absolute_input(&self.invoked, &self.cwd, self.repository.as_deref(), path);
        let name = relative(&self.cwd, &absolute);
        if self.selected.contains(&name) {
            return Ok(true);
        }
        let local = Path::new(&name);
        if !self.eligible(local) || self.extras.is_match(local) || !self.below_directory(&absolute)
        {
            return Ok(false);
        }
        if file_type(&absolute)?.is_some() {
            return Ok(false);
        }
        if self.repository.is_none() {
            return Ok(!local.iter().any(pruned));
        }
        if absolute
            .ancestors()
            .skip(1)
            .any(|parent| self.leaves.contains(parent))
        {
            return Ok(false);
        }
        git_allows(&self.cwd, &name)
    }

    fn eligible(
        &self,
        path: &Path,
    ) -> bool {
        source_extension(path).is_some() && !self.defaults.is_match(path)
    }

    fn below_directory(
        &self,
        path: &Path,
    ) -> bool {
        path.parent().is_some_and(|parent| {
            self.directories
                .iter()
                .any(|root| parent.starts_with(root) && !through_symlink(path, root))
        })
    }

    fn finish(
        mut self,
        names: BTreeSet<OsString>,
        patterns: &[ExcludePattern],
    ) -> Discovery {
        let mut used = vec![false; patterns.len()];
        let mut matches = Vec::new();
        for name in names {
            let path = Path::new(&name);
            if self.eligible(path) {
                self.extras.matches_into(path, &mut matches);
                if matches.is_empty() {
                    self.selected.insert(name);
                }
                for &index in &matches {
                    used[index] = true;
                }
            }
        }
        let mut files: Vec<_> = self.selected.iter().map(PathBuf::from).collect();
        files.sort_by_cached_key(|file| file.to_string_lossy().encode_utf16().collect::<Vec<_>>());
        let unused_excludes = patterns
            .iter()
            .zip(used)
            .filter_map(|(rule, used)| if used { None } else { Some(rule.clone()) })
            .collect();
        Discovery {
            files,
            scope: self,
            unused_excludes,
        }
    }
}

#[derive(Default)]
struct Listing {
    files: BTreeSet<OsString>,
    leaves: BTreeSet<PathBuf>,
}

pub fn discover(
    roots: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<Discovery, InputError> {
    let physical = options
        .cwd
        .canonicalize()
        .map_err(|error| InputError::at(&options.cwd, error))?;
    let invoked = std::path::absolute(&options.cwd).map_err(|e| InputError::at(&options.cwd, e))?;
    let repository = worktree(&physical)?;
    let cwd = if repository.is_some() {
        physical
    } else {
        invoked.clone()
    };
    let defaults = compile(&options.defaults)?;
    let extras = compile(&options.excludes)?;
    let resolved = resolve_roots(roots, &invoked, &cwd, repository.as_deref())?;
    let directories = resolved
        .iter()
        .filter(|root| root.is_dir)
        .map(|root| root.absolute.clone())
        .collect();
    let inputs: Vec<_> = resolved.into_iter().map(|root| root.absolute).collect();
    let listing = match &repository {
        Some(_) => git_files(&cwd, &inputs)?,
        None => Listing {
            files: walk_files(&cwd, inputs)?,
            ..Listing::default()
        },
    };
    let scope = DiscoveryScope {
        cwd,
        invoked,
        directories,
        repository,
        leaves: listing.leaves,
        selected: BTreeSet::new(),
        defaults,
        extras,
    };
    Ok(scope.finish(listing.files, &options.excludes))
}

#[derive(Debug)]
struct Exclusions {
    by_parent_depth: Vec<GlobGroup>,
}

#[derive(Debug)]
struct GlobGroup {
    set: GlobSet,
    indices: Vec<usize>,
}

impl Exclusions {
    fn group<'p>(
        &self,
        mut path: &'p Path,
    ) -> Option<(&GlobGroup, &'p Path)> {
        let mut depth = 0;
        while let Ok(rest) = path.strip_prefix("..") {
            depth += 1;
            path = rest;
        }
        self.by_parent_depth.get(depth).map(|group| (group, path))
    }

    fn is_match(
        &self,
        path: &Path,
    ) -> bool {
        self.group(path)
            .is_some_and(|(group, rest)| group.set.is_match(rest))
    }

    fn matches_into(
        &self,
        path: &Path,
        indices: &mut Vec<usize>,
    ) {
        indices.clear();
        if let Some((group, rest)) = self.group(path) {
            group.set.matches_into(rest, indices);
            for index in indices {
                *index = group.indices[*index];
            }
        }
    }
}

fn glob(pattern: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .case_insensitive(false)
        .backslash_escape(true)
        .build()
}

struct Root {
    absolute: PathBuf,
    is_dir: bool,
}

fn resolve_roots(
    roots: &[PathBuf],
    invoked: &Path,
    cwd: &Path,
    repository: Option<&Path>,
) -> Result<Vec<Root>, InputError> {
    if roots.is_empty() {
        return Ok(vec![Root {
            absolute: cwd.to_owned(),
            is_dir: true,
        }]);
    }
    roots
        .iter()
        .map(|root| {
            let absolute = absolute_input(invoked, cwd, repository, root);
            if let Some(repo) = repository
                && !absolute.starts_with(repo)
            {
                return Err(InputError::at(
                    root,
                    format!("outside the repository at {}", repo.display()),
                ));
            }
            let kind = file_type_at(root, &absolute)?
                .ok_or_else(|| InputError::at(root, "path does not exist"))?;
            Ok(Root {
                absolute,
                is_dir: kind.is_dir(),
            })
        })
        .collect()
}

fn parent_variants(pattern: &str) -> Vec<&str> {
    // A globstar can match zero directories before an explicit parent, but cannot consume `..`.
    let mut variants = vec![pattern];
    let mut current = pattern;
    while let Some((head, tail)) = current.trim_start_matches("**/").split_once('/') {
        if head.starts_with('*')
            || !glob(head).is_ok_and(|matcher| matcher.compile_matcher().is_match(".."))
        {
            break;
        }
        variants.push(tail);
        current = tail;
    }
    variants
}

fn glob_error(
    rule: &ExcludePattern,
    error: &globset::Error,
) -> InputError {
    let message = format!("invalid glob {}: {error}", rule.pattern);
    match &rule.origin {
        GlobOrigin::Config(path) => InputError::at(path, message),
        GlobOrigin::Default | GlobOrigin::Flag => InputError::message(message),
    }
}

fn compile(patterns: &[ExcludePattern]) -> Result<Exclusions, InputError> {
    let mut groups: Vec<(GlobSetBuilder, Vec<usize>)> = Vec::new();
    for (index, rule) in patterns.iter().enumerate() {
        for (depth, variant) in parent_variants(&rule.pattern).into_iter().enumerate() {
            let matcher = glob(variant).map_err(|error| glob_error(rule, &error))?;
            if depth == groups.len() {
                groups.push((GlobSetBuilder::new(), Vec::new()));
            }
            groups[depth].0.add(matcher);
            groups[depth].1.push(index);
        }
    }
    let by_parent_depth = groups
        .into_iter()
        .map(|(builder, indices)| {
            Ok(GlobGroup {
                set: builder
                    .build()
                    .map_err(|error| InputError::message(format!("invalid glob set: {error}")))?,
                indices,
            })
        })
        .collect::<Result<_, InputError>>()?;
    Ok(Exclusions { by_parent_depth })
}

pub(super) fn source_extension(path: &Path) -> Option<&str> {
    let name = path.file_name()?.as_encoded_bytes();
    let mut parts = name.rsplitn(2, |&byte| byte == b'.');
    let ext = std::str::from_utf8(parts.next()?).ok()?;
    let stem = parts.next()?;
    if stem.ends_with(b".d") && matches!(ext, "ts" | "cts" | "mts") {
        return None;
    }
    oxc_span::VALID_EXTENSIONS.contains(&ext).then_some(ext)
}

fn git(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(cwd);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_IMPLICIT_WORK_TREE",
    ] {
        command.env_remove(var);
    }
    command
}

fn worktree(cwd: &Path) -> Result<Option<PathBuf>, InputError> {
    let output = match git(cwd)
        .args(["rev-parse", "--is-inside-work-tree", "--show-cdup"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(InputError::at(cwd, format!("git rev-parse: {error}"))),
    };
    if !output.status.success() {
        return Ok(None);
    }
    let response = String::from_utf8_lossy(&output.stdout);
    let mut lines = response.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    Ok(Some(resolve(
        cwd,
        Path::new(lines.next().unwrap_or_default()),
    )))
}

fn resolve(
    base: &Path,
    path: &Path,
) -> PathBuf {
    let mut result = PathBuf::new();
    for part in base.join(path).components() {
        if part == Component::ParentDir {
            result.pop();
        } else {
            result.push(part);
        }
    }
    result
}

fn git_files(
    cwd: &Path,
    roots: &[PathBuf],
) -> Result<Listing, InputError> {
    let output = git(cwd)
        .args([
            "--literal-pathspecs",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots.iter().map(|root| relative(cwd, root)))
        .output()
        .map_err(|error| InputError::at(cwd, format!("git ls-files: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(InputError::at(
            cwd,
            format!("git ls-files: {}", message.trim_end()),
        ));
    }
    let mut listing = Listing::default();
    for record in output.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        let name = git_name(record);
        let absolute = resolve(cwd, Path::new(&name));
        if file_type(&absolute)?.is_some_and(|kind| kind.is_file()) {
            listing.files.insert(name);
        } else {
            listing.leaves.insert(absolute);
        }
    }
    Ok(listing)
}

fn git_allows(
    cwd: &Path,
    name: &OsStr,
) -> Result<bool, InputError> {
    let mut literal = OsString::from("./");
    literal.push(name);
    let output = git(cwd)
        .args(["check-ignore", "-q", "--"])
        .arg(literal)
        .output()
        .map_err(|e| InputError::at(Path::new(name), format!("git check-ignore: {e}")))?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(InputError::at(
            Path::new(name),
            format!(
                "git check-ignore ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
        )),
    }
}

fn absolute_input(
    invoked: &Path,
    cwd: &Path,
    repository: Option<&Path>,
    input: &Path,
) -> PathBuf {
    let logical = resolve(invoked, input);
    if input.is_absolute()
        && !logical.starts_with(invoked)
        && repository.is_some_and(|root| logical.starts_with(root))
    {
        return logical;
    }
    resolve(cwd, Path::new(&relative(invoked, &logical)))
}

fn relative(
    cwd: &Path,
    path: &Path,
) -> OsString {
    let Some(common) = cwd.ancestors().find(|dir| path.starts_with(dir)) else {
        return path.as_os_str().to_owned();
    };
    let up = cwd
        .strip_prefix(common)
        .unwrap()
        .iter()
        .map(|_| OsStr::new(".."));
    let down = path.strip_prefix(common).unwrap().iter();
    let parts: Vec<_> = up.chain(down).collect();
    if parts.is_empty() {
        OsString::from(".")
    } else {
        parts.join(OsStr::new("/"))
    }
}

fn through_symlink(
    path: &Path,
    boundary: &Path,
) -> bool {
    path.ancestors()
        .take_while(|part| !boundary.starts_with(part))
        .any(Path::is_symlink)
}

fn pruned(name: &OsStr) -> bool {
    name == ".git" || name == "node_modules"
}

fn walk_files(
    cwd: &Path,
    roots: Vec<PathBuf>,
) -> Result<BTreeSet<OsString>, InputError> {
    let mut pending: Vec<_> = roots
        .into_iter()
        .filter(|root| !Path::new(&relative(cwd, root)).iter().any(pruned))
        .collect();
    let mut files = BTreeSet::new();
    while let Some(path) = pending.pop() {
        let kind = fs::symlink_metadata(&path)
            .map_err(|e| InputError::at(&path, e))?
            .file_type();
        if kind.is_file() {
            files.insert(relative(cwd, &path));
        } else if kind.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| InputError::at(&path, e))? {
                let entry = entry.map_err(|e| InputError::at(&path, e))?;
                if !pruned(&entry.file_name()) {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(files)
}

fn git_name(bytes: &[u8]) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(bytes).to_owned()
    }
    #[cfg(not(unix))]
    {
        OsString::from(String::from_utf8_lossy(bytes).as_ref())
    }
}

fn file_type(path: &Path) -> Result<Option<fs::FileType>, InputError> {
    file_type_at(path, path)
}

fn file_type_at(
    reported: &Path,
    path: &Path,
) -> Result<Option<fs::FileType>, InputError> {
    fs::symlink_metadata(path)
        .map(|stat| Some(stat.file_type()))
        .or_else(|error| missing_file(reported, error))
}

fn missing_file(
    path: &Path,
    error: std::io::Error,
) -> Result<Option<fs::FileType>, InputError> {
    if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) {
        Ok(None)
    } else {
        Err(InputError::at(path, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_parent_components_without_consuming_them_with_globstars() {
        let rules = [
            "**/x.ts",
            "../**/x.ts",
            "../../**/x.ts",
            "**/../**/x.ts",
            "*/shared/x.ts",
            "??/shared/x.ts",
            r"\.\./shared/x.ts",
        ]
        .map(|p| ExcludePattern {
            pattern: p.to_owned(),
            origin: GlobOrigin::Flag,
        });
        let set = compile(&rules).unwrap();
        let mut indices = Vec::new();
        for (path, expected) in [
            ("shared/x.ts", &[0][..]),
            ("../shared/x.ts", &[1, 3, 5, 6]),
            ("../../shared/x.ts", &[2]),
            ("../../x.ts", &[2]),
            ("../../../shared/x.ts", &[]),
        ] {
            set.matches_into(Path::new(path), &mut indices);
            assert_eq!(indices, expected, "{path}");
            assert_eq!(set.is_match(Path::new(path)), !expected.is_empty());
        }
    }

    #[test]
    fn source_suffixes_apply_to_the_whole_filename() {
        assert_eq!(
            oxc_span::VALID_EXTENSIONS,
            ["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"]
        );
        for (path, expected) in [
            (".ts", true),
            ("a.d.ts.js", true),
            ("a.d.js", true),
            ("a.d.tsx", true),
            ("a.D.ts", true),
            (".d.ts", false),
            ("a.d.cts", false),
            ("a.d.mts", false),
            ("file.ts/other", false),
            ("ts", false),
            ("a.TS", false),
        ] {
            assert_eq!(
                source_extension(Path::new(path)).is_some(),
                expected,
                "{path}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_root_on_another_drive_keeps_its_absolute_path() {
        let cwd = Path::new("C:/pkg");
        let file = Path::new("D:/shared/a.ts");
        assert_eq!(absolute_input(cwd, cwd, None, file), file);
        assert_eq!(relative(cwd, file), file.as_os_str());
    }

    #[test]
    fn a_metadata_error_other_than_absence_retains_its_path() {
        let path = Path::new("unreadable.ts");
        let error = missing_file(path, ErrorKind::PermissionDenied.into()).unwrap_err();
        assert_eq!(error.path(), path);
        assert_eq!(error.side(), None);
        assert!(error.to_string().contains("unreadable.ts"));
    }

    #[test]
    fn invalid_glob_errors_preserve_pattern_and_config_origin() {
        let flag_rule = ExcludePattern {
            pattern: "src/[".to_owned(),
            origin: GlobOrigin::Flag,
        };
        let flag_err = compile(&[flag_rule]).unwrap_err();
        assert_eq!(flag_err.path(), Path::new(""));
        assert!(
            flag_err.to_string().starts_with("invalid glob src/["),
            "{flag_err}"
        );

        let default_rule = ExcludePattern {
            pattern: "src/{".to_owned(),
            origin: GlobOrigin::Default,
        };
        let default_err = compile(&[default_rule]).unwrap_err();
        assert_eq!(default_err.path(), Path::new(""));
        assert!(
            default_err.to_string().starts_with("invalid glob src/{"),
            "{default_err}"
        );

        let config_rule = ExcludePattern {
            pattern: "../[".to_owned(),
            origin: GlobOrigin::Config(PathBuf::from("sub/crapts.toml")),
        };
        let config_err = compile(&[config_rule]).unwrap_err();
        assert_eq!(config_err.path(), Path::new("sub/crapts.toml"));
        assert!(
            config_err
                .to_string()
                .starts_with("sub/crapts.toml: invalid glob ../["),
            "{config_err}"
        );
    }
}
