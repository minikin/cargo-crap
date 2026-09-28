//! Advisory triage of duplicate pairs by a `TypeSafe` System One model.
//!
//! Opt-in and annotation-only: for each pair the duplicate pass already
//! found, the model is asked what kind of duplication it is, whether it is
//! worth extracting, and whether a fix to one side would likely miss the
//! other. The answers are printed beside the pair; they never filter,
//! reorder or gate anything.
//!
//! The HTTP client (the `client` module and `run`) is compiled only with the
//! `triage` Cargo feature; decoding, requests, the cache and rendering use no
//! network and are always present.

pub mod cache;
#[cfg(feature = "triage")]
pub mod client;
pub mod request;
pub mod verdict;

#[cfg(feature = "triage")]
pub use client::{Settings, TriageError};

#[cfg(feature = "triage")]
use crate::duplicates::compare::DuplicatePair;
#[cfg(feature = "triage")]
use client::Client;
#[cfg(feature = "triage")]
use rayon::prelude::*;
#[cfg(feature = "triage")]
use verdict::Verdict;

/// Ask the model about every pair, in parallel, returning one verdict per
/// pair in the pairs' order.
///
/// All or nothing: a report where some pairs carry a verdict and others do
/// not invites reading the absence as a verdict, so one pair failing — after
/// its retries — fails the run. No pairs means no requests, even without a
/// key.
///
/// # Errors
///
/// When the key is missing, a side's source cannot be read back, or any
/// pair's request fails or answers with something that does not decode.
#[cfg(feature = "triage")]
pub fn run(
    pairs: &[DuplicatePair],
    settings: &Settings,
) -> Result<Vec<Verdict>, TriageError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let client = Client::new(settings)?;
    // Its own pool: the work is waiting on the network, not computing, so it
    // is sized to requests rather than cores, and its waits and back-off
    // sleeps never occupy a thread of the pool the scan runs on.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(REQUEST_THREADS)
        .build()
        .map_err(|e| TriageError::Threads(e.to_string()))?;
    pool.install(|| {
        pairs
            .par_iter()
            .map(|pair| judge(&client, pair, &settings.model))
            .collect()
    })
}

/// How many requests a run keeps in flight at once.
#[cfg(feature = "triage")]
const REQUEST_THREADS: usize = 8;

/// One pair: build its request from the source on disk, send it, decode it.
#[cfg(feature = "triage")]
fn judge(
    client: &Client,
    pair: &DuplicatePair,
    model: &str,
) -> Result<Verdict, TriageError> {
    let body = request::build(pair, model).map_err(TriageError::Source)?;
    let answer = client.evaluate(&body.to_string())?;
    Verdict::decode(&answer).map_err(TriageError::Decode)
}
