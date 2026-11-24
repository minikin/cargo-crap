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

#[cfg(feature = "triage")]
pub use cargo_crap_core::duplicates::triage::{Settings, TriageError, Triaged, client};
pub use cargo_crap_core::duplicates::triage::{budget, cache, verdict};
pub mod provider;
pub mod request;

#[cfg(feature = "triage")]
use crate::duplicates::compare::DuplicatePair;

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
    cargo_crap_core::duplicates::triage::run(pairs, settings, &request::QUESTIONS, |pair| {
        let (first, second) = request::sources(pair)?;
        let key = cache::CacheKey::new(
            settings.provider,
            &first,
            &second,
            &settings.model,
            request::QUESTION_SET_VERSION,
        );
        Ok(cargo_crap_core::duplicates::triage::Excerpts { first, second, key })
    })
}
