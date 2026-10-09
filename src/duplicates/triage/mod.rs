//! Advisory triage of duplicate pairs by a model behind a typed-answer API.
//!
//! Opt-in and annotation-only: for each pair the duplicate pass already
//! found, the model is asked what kind of duplication it is, whether it is
//! worth extracting, and whether a fix to one side would likely miss the
//! other. The answers are printed beside the pair; they never filter,
//! reorder or gate anything.
//!
//! Which API is asked is a [`provider`]: it owns the wire shape, the
//! endpoint and the environment, and everything else here is shared. The
//! HTTP client (the `client` module and `run`) is compiled only with the
//! `triage` Cargo feature. Providers, decoding, requests, the cache and
//! rendering use no network and are always present.

pub mod cache;
#[cfg(feature = "triage")]
pub mod client;
pub mod provider;
pub mod request;
pub mod verdict;

#[cfg(feature = "triage")]
pub use client::{Settings, TriageError};

#[cfg(feature = "triage")]
use crate::duplicates::compare::DuplicatePair;
#[cfg(feature = "triage")]
use cache::{Cache, CacheKey};
#[cfg(feature = "triage")]
use client::Client;
#[cfg(feature = "triage")]
use provider::Provider;
#[cfg(feature = "triage")]
use rayon::prelude::*;
#[cfg(feature = "triage")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "triage")]
use verdict::Verdict;

/// Ask the model about every pair, in parallel, returning one verdict per
/// pair in the pairs' order.
///
/// All or nothing: a report where some pairs carry a verdict and others do
/// not invites reading the absence as a verdict, so one pair failing (after
/// its retries) fails the run. No pairs means no requests, even without a
/// key.
///
/// A pair whose two bodies were judged before, by the same model and
/// question set, is answered from the cache. Every fresh verdict is cached
/// as soon as it arrives, including in a run that later fails, so a retry
/// pays only for the pairs that were not answered. A cache that cannot be
/// written costs one warning and nothing else.
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
    // Built now, needed only on a cache miss: a run whose every pair is
    // cached needs neither the API nor its key.
    let client = Client::new(settings);
    // Its own pool: the work is waiting on the network, not computing, so it
    // is sized to requests rather than cores, and its waits and back-off
    // sleeps never occupy a thread of the pool the scan runs on.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(REQUEST_THREADS)
        .build()
        .map_err(|e| TriageError::Threads(e.to_string()))?;
    let cache = settings.cache_dir.as_ref().map(Cache::new);
    let run = Run {
        client: client.as_ref(),
        provider: settings.provider,
        model: &settings.model,
        cache: cache.as_ref(),
        warned: AtomicBool::new(false),
    };
    pool.install(|| pairs.par_iter().map(|pair| run.judge(pair)).collect())
}

/// What every pair of one run shares.
#[cfg(feature = "triage")]
struct Run<'a> {
    /// `None` without an API key, which is all a missing client can mean.
    /// Only a cache miss needs it.
    client: Option<&'a Client>,
    /// Whose wire shape the requests are written in and the answers read in.
    provider: &'a dyn Provider,
    model: &'a str,
    cache: Option<&'a Cache>,
    /// Set once a cache write has failed and been reported.
    warned: AtomicBool,
}

/// How many requests a run keeps in flight at once.
#[cfg(feature = "triage")]
const REQUEST_THREADS: usize = 8;

#[cfg(feature = "triage")]
impl Run<'_> {
    /// One pair: answered from the cache when its bodies were judged before,
    /// otherwise asked from the source on disk and cached.
    fn judge(
        &self,
        pair: &DuplicatePair,
    ) -> Result<Verdict, TriageError> {
        let (source_a, source_b) = request::sources(pair).map_err(TriageError::Source)?;
        let key = CacheKey::new(
            self.provider,
            &source_a,
            &source_b,
            self.model,
            request::QUESTION_SET_VERSION,
        );
        if let Some(verdict) = self.cache.and_then(|cache| cache.get(key)) {
            return Ok(verdict);
        }
        let client = self.client.ok_or(TriageError::MissingKey {
            var: self.provider.key_var(),
        })?;
        let body = request::body(self.provider, pair, &source_a, &source_b, self.model);
        let verdict = self
            .provider
            .decode(&client.evaluate(&body.to_string())?)
            .and_then(Verdict::from_answers)
            .map_err(|error| TriageError::Decode {
                api: self.provider.display_name(),
                error,
            })?;
        self.remember(key, &verdict);
        Ok(verdict)
    }

    /// Cache `verdict`; on the first failure, say so once and carry on.
    fn remember(
        &self,
        key: CacheKey,
        verdict: &Verdict,
    ) {
        let Some(cache) = self.cache else { return };
        if let Err(e) = cache.put(key, verdict)
            && !self.warned.swap(true, Ordering::Relaxed)
        {
            eprintln!(
                "warning: could not write the triage cache ({e}); \
                 verdicts will be asked for again next run"
            );
        }
    }
}
