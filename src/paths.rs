//! Path normalization and suffix matching shared by report consumers.

use std::path::Path;

/// A path with its separators unified, so a baseline written on Windows
/// matches a run on Unix.
pub(crate) fn path_key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Number of trailing path components two files share, after forward-slash
/// normalization. `0` means not even the filename matches.
fn common_suffix_len(
    a: &Path,
    b: &Path,
) -> usize {
    let a = path_key(a);
    let b = path_key(b);
    a.split('/')
        .rev()
        .zip(b.split('/').rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// For `from`, find the index in `candidates` whose file shares the longest
/// common path suffix with it. Returns `None` when no candidate shares at
/// least the filename, or when the best score is tied (ambiguous).
pub(crate) fn unique_best_by_suffix<T>(
    from: &Path,
    candidates: &[T],
    file_of: impl Fn(&T) -> &Path,
) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (candidate idx, score)
    let mut tied = false;
    for (i, c) in candidates.iter().enumerate() {
        let score = common_suffix_len(from, file_of(c));
        if score == 0 {
            continue;
        }
        match best {
            None => best = Some((i, score)),
            Some((_, s)) => match score.cmp(&s) {
                std::cmp::Ordering::Equal => tied = true,
                std::cmp::Ordering::Greater => {
                    best = Some((i, score));
                    tied = false;
                },
                std::cmp::Ordering::Less => {},
            },
        }
    }
    match (best, tied) {
        (Some((i, _)), false) => Some(i),
        _ => None,
    }
}
