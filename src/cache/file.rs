//! File mechanics shared by every on-disk cache: a write that a concurrent
//! reader never sees half done, and a read that survives the moment Windows
//! denies access while that write replaces the file.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Numbers the temporary files [`write_atomic`] writes before renaming them.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// How many times [`read_retrying`] tries a read that is denied access.
const READ_ATTEMPTS: u32 = 5;

/// The pause between those tries.
const RETRY_DELAY: Duration = Duration::from_millis(2);

/// `read`'s result, tried again while it is denied access.
///
/// On Windows a read that lands while [`write_atomic`] replaces the file is
/// denied access for a moment, while the old file is being deleted. Without
/// the retry that read would count as a miss. Any other error, a missing
/// file included, returns at once.
///
/// # Errors
///
/// The last error `read` returned.
pub fn read_retrying<T>(mut read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for _ in 1..READ_ATTEMPTS {
        match read() {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                std::thread::sleep(RETRY_DELAY);
            },
            result => return result,
        }
    }
    read()
}

/// Write `bytes` to `dir/name`, creating `dir` if needed. `name` is a file
/// name, never a path: the temporary file and the rename stay in `dir`.
///
/// The bytes go to a temporary file in `dir` first, then are renamed into
/// place: a rename within one directory is atomic, so a concurrent reader
/// (a rayon worker, or another run sharing `target/`) sees the old file or
/// the new one, never a torn file. The temporary name is unique per process
/// and per write, and a failed write removes it.
///
/// # Errors
///
/// When `dir` cannot be created or the file cannot be written or renamed.
pub fn write_atomic(
    dir: &Path,
    name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, dir.join(name)))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A reader that fails with `errors` in turn, then reads `"entry"`, and
    /// counts its calls.
    fn scripted_read(
        errors: Vec<io::ErrorKind>
    ) -> (
        impl FnMut() -> io::Result<String>,
        std::rc::Rc<std::cell::Cell<usize>>,
    ) {
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = std::rc::Rc::clone(&calls);
        let read = move || {
            let n = counter.get();
            counter.set(n + 1);
            errors.get(n).map_or_else(
                || Ok("entry".to_owned()),
                |&kind| Err(io::Error::from(kind)),
            )
        };
        (read, calls)
    }

    #[test]
    fn a_read_denied_access_while_the_entry_is_replaced_is_tried_again() {
        // Windows denies a read that lands while the entry is being replaced.
        let (read, calls) = scripted_read(vec![io::ErrorKind::PermissionDenied; 2]);
        assert_eq!(read_retrying(read).expect("the third try reads"), "entry");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_missing_entry_is_not_tried_again() {
        // A miss is the common case on a first run, so it costs one read.
        let (read, calls) = scripted_read(vec![io::ErrorKind::NotFound]);
        assert_eq!(
            read_retrying(read).expect_err("missing").kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_read_still_denied_after_the_last_try_is_a_miss() {
        let (read, calls) = scripted_read(vec![io::ErrorKind::PermissionDenied; 99]);
        assert_eq!(
            read_retrying(read).expect_err("still denied").kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(calls.get(), READ_ATTEMPTS as usize);
    }

    /// The names in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .expect("the directory")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_atomic_creates_the_directory_and_leaves_only_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("target/cargo-crap");
        write_atomic(&nested, "complexity.json", b"{}").expect("writable");
        assert_eq!(
            std::fs::read(nested.join("complexity.json")).expect("written"),
            b"{}"
        );
        assert_eq!(
            names(&nested),
            ["complexity.json"],
            "no temporary file left"
        );
    }

    #[test]
    fn write_atomic_replaces_an_existing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_atomic(dir.path(), "f", b"old").expect("writable");
        write_atomic(dir.path(), "f", b"new").expect("writable");
        assert_eq!(
            std::fs::read(dir.path().join("f")).expect("written"),
            b"new"
        );
        assert_eq!(names(dir.path()), ["f"]);
    }

    #[test]
    fn a_failed_write_atomic_leaves_no_temporary_file() {
        // The destination is a non-empty directory, so the rename fails.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("f/inside")).expect("mkdir");
        write_atomic(dir.path(), "f", b"bytes").expect_err("cannot replace a directory");
        assert_eq!(names(dir.path()), ["f"], "the temporary file is removed");
    }

    #[test]
    fn write_atomic_fails_when_the_directory_cannot_be_created() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("file"), b"").expect("write");
        write_atomic(&dir.path().join("file/sub"), "f", b"x").expect_err("a file is in the way");
    }

    proptest! {
        #[test]
        fn write_atomic_then_read_returns_the_bytes(
            bytes in proptest::collection::vec(any::<u8>(), 0..512),
        ) {
            let dir = tempfile::tempdir().expect("temp dir");
            write_atomic(dir.path(), "f", &bytes).expect("writable");
            let read = read_retrying(|| std::fs::read(dir.path().join("f")));
            prop_assert_eq!(read.expect("readable"), bytes);
        }
    }
}
