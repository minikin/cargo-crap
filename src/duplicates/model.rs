//! Duplicate function locations and structural fingerprints.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// A structural fingerprint: the hash of one normalized subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub u64);

/// Where one function is, and what it is called.
///
/// Carried apart from the fingerprints because it is all a reported pair
/// needs: the sets answer "how alike", the location answers "which two", and
/// only the second survives into the report.
///
/// The field order is the ordering. File and start line alone are not a total
/// order — `fn a() {} fn b() {}` on one line share both, and a tie there would
/// let the order functions happened to be discovered in leak into the report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    /// File the function was found in.
    pub file: PathBuf,
    /// 1-indexed first line, inclusive.
    pub start_line: usize,
    /// 1-indexed last line, inclusive.
    pub end_line: usize,
    /// Function or method name, as written.
    pub name: String,
}

/// One function, located and fingerprinted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionPrint {
    /// Where it is and what it is called.
    pub location: Location,
    /// Number of nodes in the normalized tree.
    pub node_count: usize,
    /// Every distinct normalized subtree shape in the function.
    pub prints: BTreeSet<Fingerprint>,
}
