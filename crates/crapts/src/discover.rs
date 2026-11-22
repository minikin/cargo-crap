use std::collections::{BTreeMap, BTreeSet};
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
    pub base_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobOrigin {
    Default,
    Config(PathBuf),
    Flag,
}

impl ExcludePattern {
    #[must_use]
    pub fn unused_warning(&self) -> String {
        let pattern = &self.pattern;
        match &self.origin {
            GlobOrigin::Config(file) => format!(
                "{}: exclude '{pattern}' excluded no file; globs are relative to the config directory",
                file.display()
            ),
            GlobOrigin::Default | GlobOrigin::Flag => format!(
                "--exclude '{pattern}' excluded no file; globs are relative to the current directory"
            ),
        }
    }
}

impl DiscoveryOptions {
    pub(crate) fn validate(
        cwd: &Path,
        patterns: &[ExcludePattern],
    ) -> Result<(), InputError> {
        compile(cwd, patterns).map(|_| ())
    }

    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let cwd = cwd.into();
        let base_dir = std::path::absolute(&cwd).unwrap_or_else(|_| PathBuf::from("."));
        let defaults = DEFAULT_EXCLUDES
            .iter()
            .map(|&pattern| ExcludePattern {
                pattern: pattern.to_owned(),
                origin: GlobOrigin::Default,
                base_dir: base_dir.clone(),
            })
            .collect();
        Self {
            cwd,
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
    worktree: Worktree,
    directories: Vec<PathBuf>,
    leaves: BTreeSet<PathBuf>,
    selected: BTreeSet<OsString>,
    defaults: Exclusions,
    extras: Exclusions,
}

impl DiscoveryScope {
    #[must_use]
    pub fn repo_prefix(&self) -> &Path {
        &self.worktree.prefix
    }

    /// Rules retain their configured base even when the directory no longer exists.
    pub fn exclusions(&self) -> impl Iterator<Item = &ExcludePattern> {
        self.defaults.patterns.iter().chain(&self.extras.patterns)
    }

    /// # Errors
    /// Filesystem lookup or Git ignore checks can fail for paths absent from the discovery result.
    pub fn keeps(
        &self,
        path: &Path,
    ) -> Result<bool, InputError> {
        let absolute = absolute_input(
            &self.invoked,
            &self.cwd,
            self.worktree.root.as_deref(),
            path,
        );
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
        if !self.worktree.indexed {
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
    ) -> Discovery {
        let mut used = vec![false; self.extras.patterns.len()];
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
        let unused_excludes = self
            .extras
            .patterns
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

/// # Errors
/// Root errors take precedence over invalid globs, which still fail an empty scan.
/// Filesystem traversal and Git listing failures also return an input error.
pub fn discover(
    roots: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<Discovery, InputError> {
    let physical = options
        .cwd
        .canonicalize()
        .map_err(|error| InputError::at(&options.cwd, error))?;
    let invoked = std::path::absolute(&options.cwd).map_err(|e| InputError::at(&options.cwd, e))?;
    let worktree = Worktree::probe(&physical)?;
    let cwd = if worktree.root.is_some() {
        physical
    } else {
        invoked.clone()
    };
    let resolved = resolve_roots(roots, &invoked, &cwd, worktree.root.as_deref())?;
    let defaults = compile(&invoked, &options.defaults)?;
    let extras = compile(&invoked, &options.excludes)?;
    let directories = resolved
        .iter()
        .filter(|root| root.is_dir)
        .map(|root| root.absolute.clone())
        .collect();
    let inputs: Vec<_> = resolved.into_iter().map(|root| root.absolute).collect();
    let listing = if worktree.indexed {
        git_files(&cwd, &inputs)?
    } else {
        Listing {
            files: walk_files(&cwd, inputs)?,
            ..Listing::default()
        }
    };
    let scope = DiscoveryScope {
        cwd,
        invoked,
        directories,
        worktree,
        leaves: listing.leaves,
        selected: BTreeSet::new(),
        defaults,
        extras,
    };
    Ok(scope.finish(listing.files))
}

#[derive(Debug)]
struct Exclusions {
    cwd: PathBuf,
    patterns: Vec<ExcludePattern>,
    groups: BTreeMap<PathBuf, ParentGroups>,
}

#[derive(Debug)]
struct ParentGroups {
    by_parent_depth: Vec<GlobGroup>,
}

#[derive(Debug)]
struct GlobGroup {
    set: GlobSet,
    indices: Vec<usize>,
}

impl ParentGroups {
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
}

impl Exclusions {
    fn matching_groups(
        &self,
        path: &Path,
    ) -> impl Iterator<Item = (&GlobGroup, PathBuf)> {
        let file = resolve(&self.cwd, path);
        self.groups.iter().filter_map(move |(base, groups)| {
            let name = relative(base, &file);
            let (group, rest) = groups.group(Path::new(&name))?;
            Some((group, rest.to_owned()))
        })
    }

    fn is_match(
        &self,
        path: &Path,
    ) -> bool {
        self.matching_groups(path)
            .any(|(group, rest)| group.set.is_match(rest))
    }

    fn matches_into(
        &self,
        path: &Path,
        indices: &mut Vec<usize>,
    ) {
        indices.clear();
        for (group, rest) in self.matching_groups(path) {
            indices.extend(
                group
                    .set
                    .matches(rest)
                    .into_iter()
                    .map(|index| group.indices[index]),
            );
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
            let kind =
                file_type_at(root, &absolute)?.ok_or_else(|| InputError::missing_root(root))?;
            Ok(Root {
                absolute,
                is_dir: kind.is_dir(),
            })
        })
        .collect()
}

fn has_brace_group(head: &str) -> bool {
    let mut chars = head.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => return true,
            '\\' => {
                chars.next();
            },
            '[' => {
                // Globset allows a literal ']' as the first class member after an optional negation.
                chars.next_if(|c| matches!(c, '!' | '^'));
                chars.next();
                chars.by_ref().find(|&c| c == ']');
            },
            _ => {},
        }
    }
    false
}

fn parent_variants(pattern: &str) -> Vec<&str> {
    // A globstar can match zero directories before an explicit parent, but cannot consume `..`.
    let mut variants = vec![pattern];
    let mut current = pattern;
    while let Some((head, tail)) = current.trim_start_matches("**/").split_once('/') {
        if head.starts_with('*')
            || head.starts_with(".*")
            || has_brace_group(head)
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

type GlobBuilders = Vec<(GlobSetBuilder, Vec<usize>)>;

fn compile(
    cwd: &Path,
    patterns: &[ExcludePattern],
) -> Result<Exclusions, InputError> {
    let mut by_base = BTreeMap::<PathBuf, GlobBuilders>::new();
    for (index, rule) in patterns.iter().enumerate() {
        let groups = by_base.entry(resolve(cwd, &rule.base_dir)).or_default();
        for (depth, variant) in parent_variants(&rule.pattern).into_iter().enumerate() {
            let matcher = glob(variant).map_err(|error| glob_error(rule, &error))?;
            if depth == groups.len() {
                groups.push((GlobSetBuilder::new(), Vec::new()));
            }
            groups[depth].0.add(matcher);
            groups[depth].1.push(index);
        }
    }
    let groups = by_base
        .into_iter()
        .map(|(base, builders)| Ok((base, build_groups(builders)?)))
        .collect::<Result<_, InputError>>()?;
    Ok(Exclusions {
        cwd: cwd.to_owned(),
        groups,
        patterns: patterns.to_owned(),
    })
}

fn build_groups(builders: GlobBuilders) -> Result<ParentGroups, InputError> {
    let by_parent_depth = builders
        .into_iter()
        .map(|(builder, indices)| {
            let set = builder
                .build()
                .map_err(|error| InputError::message(format!("invalid glob set: {error}")))?;
            Ok(GlobGroup { set, indices })
        })
        .collect::<Result<_, InputError>>()?;
    Ok(ParentGroups { by_parent_depth })
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

#[derive(Debug, Default)]
pub(crate) struct Worktree {
    pub(crate) root: Option<PathBuf>,
    prefix: PathBuf,
    indexed: bool,
}

impl Worktree {
    pub(crate) fn probe(cwd: &Path) -> Result<Self, InputError> {
        let top = match git(cwd).args(["rev-parse", "--show-toplevel"]).output() {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => return Self::filesystem(cwd),
            Err(error) => {
                return Err(InputError::at(
                    cwd,
                    format!("git rev-parse --show-toplevel: {error}"),
                ));
            },
        };
        if !top.status.success() {
            return Ok(Self::default());
        }
        let root = PathBuf::from(git_name(git_line(&top.stdout)));
        let root = root.canonicalize().map_err(|error| {
            InputError::at(&root, format!("git rev-parse --show-toplevel: {error}"))
        })?;
        Ok(Self {
            root: Some(root),
            prefix: git_prefix(cwd)?,
            indexed: true,
        })
    }

    fn filesystem(cwd: &Path) -> Result<Self, InputError> {
        for dir in cwd.ancestors() {
            if file_type(&dir.join(".git"))?.is_some_and(|kind| kind.is_dir() || kind.is_file()) {
                return Ok(Self {
                    root: Some(dir.to_owned()),
                    prefix: cwd
                        .strip_prefix(dir)
                        .expect("an ancestor is a prefix")
                        .to_owned(),
                    indexed: false,
                });
            }
        }
        Ok(Self::default())
    }
}

fn git_line(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

fn git_prefix(cwd: &Path) -> Result<PathBuf, InputError> {
    let command = "git rev-parse --is-inside-work-tree --show-prefix";
    let output = git(cwd)
        .args(["rev-parse", "--is-inside-work-tree", "--show-prefix"])
        .output()
        .map_err(|error| InputError::at(cwd, format!("{command}: {error}")))?;
    if !output.status.success() {
        return Err(InputError::at(
            cwd,
            format!(
                "{command}: {}",
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
        ));
    }
    let prefix = output
        .stdout
        .strip_prefix(b"true\n")
        .ok_or_else(|| InputError::at(cwd, format!("{command}: expected a worktree prefix")))?;
    Ok(PathBuf::from(git_name(git_line(prefix))))
}

pub(crate) fn resolve(
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
    let Some((depth, tail)) = cwd
        .ancestors()
        .enumerate()
        .find_map(|(depth, dir)| path.strip_prefix(dir).ok().map(|tail| (depth, tail)))
    else {
        return path.as_os_str().to_owned();
    };
    let up = std::iter::repeat_n(OsStr::new(".."), depth);
    let parts: Vec<_> = up.chain(tail).collect();
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
            "**/.*/**",
            ".*/**",
            "{..,x}/**",
        ]
        .map(|p| ExcludePattern {
            pattern: p.to_owned(),
            origin: GlobOrigin::Flag,
            base_dir: PathBuf::from("."),
        });
        let set = compile(Path::new("/old/checkout/pkg"), &rules).unwrap();
        let mut indices = Vec::new();
        for (path, expected) in [
            ("shared/x.ts", &[0][..]),
            (".cache/y.ts", &[7, 8]),
            ("shared/.cache/y.ts", &[7]),
            ("x/y.ts", &[9]),
            ("../shared/x.ts", &[1, 3, 5, 6]),
            ("../shared/.cache/y.ts", &[]),
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
    fn only_explicit_glob_heads_can_match_a_parent_component() {
        assert!(!has_brace_group(r"\{a,b\}"));
        for (head, expected) in [
            ("??", true),
            ("?.", true),
            (".?", true),
            ("[.][.]", true),
            ("?*", true),
            ("[.]*", true),
            (r"\.\.", true),
            ("[.{][.]", true),
            ("[.a{].", true),
            ("[^]{].", true),
            ("*", false),
            (".*", false),
            ("{..,foo}", false),
            ("?{.,x}", false),
        ] {
            let pattern = format!("{head}/shared/**");
            let variants = parent_variants(&pattern);
            assert_eq!(variants.contains(&"shared/**"), expected, "{head}");
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
            base_dir: PathBuf::from("."),
        };
        let flag_err = compile(Path::new("/old/checkout/pkg"), &[flag_rule]).unwrap_err();
        assert_eq!(flag_err.path(), Path::new(""));
        assert!(
            flag_err.to_string().starts_with("invalid glob src/["),
            "{flag_err}"
        );

        let default_rule = ExcludePattern {
            pattern: "src/{".to_owned(),
            origin: GlobOrigin::Default,
            base_dir: PathBuf::from("."),
        };
        let default_err = compile(Path::new("/old/checkout/pkg"), &[default_rule]).unwrap_err();
        assert_eq!(default_err.path(), Path::new(""));
        assert!(
            default_err.to_string().starts_with("invalid glob src/{"),
            "{default_err}"
        );

        let config_rule = ExcludePattern {
            pattern: "../[".to_owned(),
            origin: GlobOrigin::Config(PathBuf::from("sub/crapts.toml")),
            base_dir: PathBuf::from("."),
        };
        let config_err = compile(Path::new("/old/checkout/pkg"), &[config_rule]).unwrap_err();
        assert_eq!(config_err.path(), Path::new("sub/crapts.toml"));
        assert!(
            config_err
                .to_string()
                .starts_with("sub/crapts.toml: invalid glob ../["),
            "{config_err}"
        );
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn filesystem_boundaries_recognize_the_nearest_git_directory_or_gitfile() {
        let repo = tempfile::tempdir().unwrap();
        let nested = repo.path().join("outer/packages/app");
        fs::create_dir_all(&nested).unwrap();
        let outer = repo.path().join("outer");
        fs::create_dir(repo.path().join(".git")).unwrap();
        fs::write(outer.join(".git"), "gitdir: ../metadata").unwrap();
        let tree = Worktree::filesystem(&nested).unwrap();
        assert_eq!(tree.root.as_deref(), Some(outer.as_path()));
        assert_eq!(tree.prefix, Path::new("packages/app"));
        fs::remove_file(outer.join(".git")).unwrap();
        let tree = Worktree::filesystem(&nested).unwrap();
        assert_eq!(tree.root.as_deref(), Some(repo.path()));
        assert_eq!(tree.prefix, Path::new("outer/packages/app"));
        fs::remove_dir(repo.path().join(".git")).unwrap();
        let tree = Worktree::filesystem(&nested).unwrap();
        assert_eq!(tree.root, None);
        assert_eq!(tree.prefix, Path::new(""));
    }

    proptest! {
        #[test]
        fn parent_config_globs_rebase_across_nested_invocations(
            dir in "[a-z]{1,6}", depth in 1usize..5, keep in any::<bool>(),
        ) {
            let root = std::env::temp_dir().join("historical");
            let nested = (0..depth).fold(root.join(&dir), |cwd, _| cwd.join("child"));
            let rule = ExcludePattern { pattern: format!("{dir}/**/gen/*.ts"), origin: GlobOrigin::Config(root.join(".crapts.toml")), base_dir: root };
            let set = compile(&nested, &[rule]).unwrap();
            prop_assert_eq!(set.is_match(Path::new(if keep { "src/a.ts" } else { "gen/a.ts" })), !keep);
        }
    }
}
