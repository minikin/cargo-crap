# Changelog

This project follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- TypeSafe and OpenAI duplicate triage share question encoding, verdict decoding, cache keys and request budgets. The optional `triage` feature enables the blocking HTTP client.
- `cache_file` exposes atomic replacement and read retries. `duplicates::fnv` supplies stable 64-bit FNV-1a hashes.

- Shared coverage, complexity scoring, and report rendering.
