# Changelog

This project follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.2.0] - 2026-11-25

### Added

- The optional `triage` feature enables the blocking HTTP client for TypeSafe
  and OpenAI. Both providers share verdict decoding and request budgets.
  `run_with_cache` accepts a front end's cache and warning handler.
- `cache::Retention` sets age and entry limits. Apply them with
  `Cache::with_retention` and run cleanup with `Cache::prune`.
- `delta::DuplicateHistory` and `CraptsBaseline::duplicate_history` expose
  saved groups and family fingerprints. `model::DuplicateContext` and
  `model::Family` record the producer's coordinates and numbered siblings.
  `novelty::compare_crapts` uses them to identify new, grown and resolved groups.
- `Scope::historical_path` maps saved paths into the current scope.
  `PreparedBaseline::duplicate_record` exposes the scoped history.
  `report::normalize_crapts_path` normalizes paths for crapts reports.
  `Format::carries_duplicates` identifies formats that show groups.
- `cache_file` exposes atomic replacement and read retries.
  `duplicates::fnv` supplies stable 64-bit FNV-1a hashes.
- `gate::GateOutcome.signal_regressions` reports the post-filter gated
  structural signal regression count (`Option<usize>`) when signals and a
  measured baseline are both present.
- Shared coverage, complexity scoring, and report rendering.

### Changed

- **Breaking library API.** `QuestionSet` fields are private. Construct one
  with `QuestionSet::new` and read it through `kind()`, `worth()` and
  `divergence()`. Providers can inspect question IDs, options and score levels.
- **Breaking library API.** Core 0.2.0 adds `CoverageKind::Unmeasured` and
  marks the enum `#[non_exhaustive]`. Downstream matches need a wildcard arm.
- **Breaking library API.** `RenderExtras::duplicate_context` carries the
  producer's duplicate history metadata. `RenderExtras::duplicate_row_limit`
  and `RenderOptions::duplicate_row_limit` cap PR comment rows while keeping
  full group and budget counts. Set these fields to `None` in existing literals.
- `Excerpts.locations` accepts explicit source locations for both sides.
  Set `locations: None` in existing literals to use the members' line ranges.

[Unreleased]: https://github.com/minikin/cargo-crap/compare/cargo-crap-core-v0.2.0...HEAD
[0.2.0]: https://github.com/minikin/cargo-crap/releases/tag/cargo-crap-core-v0.2.0
