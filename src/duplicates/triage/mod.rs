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

pub mod budget;
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

/// Answer every pair, in parallel, returning one verdict per pair in the
/// pairs' order and how many pairs the budget left unasked.
///
/// Of the pairs the cache cannot answer, the first `settings.max_requests`,
/// in the order given, are asked, and the others get `None`. A key is
/// needed only when something is asked, so a budget of 0 never needs one.
///
/// Failure is all or nothing: one asked pair failing (after its retries)
/// fails the run, so no verdict stands beside an error. No pairs means no
/// requests, even without a key.
///
/// A pair whose two bodies were judged before, by the same model and
/// question set, is answered from the cache, for free. Every fresh verdict is cached
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
) -> Result<Triaged, TriageError> {
    if pairs.is_empty() {
        return Ok(Triaged {
            verdicts: Vec::new(),
            skipped: 0,
        });
    }
    // Built now, needed only when something is asked: a run whose pairs are
    // all cached, or past its budget, needs neither the API nor its key.
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
    pool.install(|| run.answer(pairs, settings.max_requests))
}

/// One pair as the cache saw it: its two bodies, their key, and the verdict
/// cached for them, if any.
#[cfg(feature = "triage")]
struct Lookup {
    sources: (String, String),
    key: CacheKey,
    cached: Option<Verdict>,
}

/// What a run answered: one verdict per pair, `None` for a pair the budget
/// left unasked, and how many those are.
#[cfg(feature = "triage")]
#[derive(Debug, Clone, PartialEq)]
pub struct Triaged {
    pub verdicts: Vec<Option<Verdict>>,
    pub skipped: usize,
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
    /// Every pair answered: from the cache where it can, then by asking
    /// about the misses `max_requests` affords, all in parallel.
    fn answer(
        &self,
        pairs: &[DuplicatePair],
        max_requests: Option<u32>,
    ) -> Result<Triaged, TriageError> {
        let looked = pairs
            .par_iter()
            .map(|pair| self.look_up(pair))
            .collect::<Result<Vec<Lookup>, TriageError>>()?;
        let answered: Vec<bool> = looked.iter().map(|l| l.cached.is_some()).collect();
        let plan = budget::plan(&answered, max_requests);
        let asked = plan
            .ask
            .par_iter()
            .map(|&n| self.ask(&pairs[n], &looked[n]))
            .collect::<Result<Vec<Verdict>, TriageError>>()?;
        let mut verdicts: Vec<Option<Verdict>> = looked.into_iter().map(|l| l.cached).collect();
        for (&n, verdict) in plan.ask.iter().zip(asked) {
            verdicts[n] = Some(verdict);
        }
        Ok(Triaged {
            verdicts,
            skipped: plan.skipped,
        })
    }

    /// One pair's bodies, read back from disk, and the verdict cached for
    /// them when they were judged before by the same model and questions.
    fn look_up(
        &self,
        pair: &DuplicatePair,
    ) -> Result<Lookup, TriageError> {
        let (source_a, source_b) = request::sources(pair).map_err(TriageError::Source)?;
        let key = CacheKey::new(
            self.provider,
            &source_a,
            &source_b,
            self.model,
            request::QUESTION_SET_VERSION,
        );
        Ok(Lookup {
            cached: self.cache.and_then(|cache| cache.get(key)),
            sources: (source_a, source_b),
            key,
        })
    }

    /// Ask about one pair the cache could not answer, and cache the verdict.
    fn ask(
        &self,
        pair: &DuplicatePair,
        lookup: &Lookup,
    ) -> Result<Verdict, TriageError> {
        let client = self.client.ok_or(TriageError::MissingKey {
            var: self.provider.key_var(),
        })?;
        let (source_a, source_b) = &lookup.sources;
        let body = request::body(self.provider, pair, source_a, source_b, self.model);
        let verdict = self
            .provider
            .decode(&client.evaluate(&body.to_string())?)
            .and_then(Verdict::from_answers)
            .map_err(|error| TriageError::Decode {
                api: self.provider.display_name(),
                error,
            })?;
        self.remember(lookup.key, &verdict);
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
