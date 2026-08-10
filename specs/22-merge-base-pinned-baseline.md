# Spec 22 — Merge-base-pinned CI baseline

**Status:** Implemented
**Effort:** Small
**Module:** `.github/workflows/ci.yml` (CI infrastructure only — no Rust code)

## Context

The self-score job resolves its regression baseline as "the newest
successful main CI run that carries a non-expired `crap-baseline`
artifact". The PR analysis, however, runs against the PR's **merge
preview** — the PR branch merged into the *current* main tip.

Those two references can disagree. Every PR CI run that starts between a
merge to main and the completion of that merge's own main run compares
"code that already contains the merge" against "a baseline that predates
it". Observed on PR #51 immediately after PR #50 merged: a release PR
touching only `Cargo.toml`/`CHANGELOG.md`/`README.md` reported PR #50's
four new functions as `★ 4 new`.

The cosmetic case is misleading; the dangerous case is worse: if the
previously merged PR legitimately changed scores, the stale baseline
reports those changes as regressions on an unrelated PR, and the
`--fail-regression` gate fails spuriously.

The fix is to pin the baseline lookup to the main commit that is actually
part of the merge preview (its first parent), instead of "latest
successful main run".

---

## Acceptance Tests

Scenarios describe observable CI behaviour; "MAIN_SHA" is the first
parent of the merge-preview commit — the exact main tip the analysis
includes.

### Scenario: Baseline comes from the merge preview's own main parent

```
Given a PR whose merge preview is based on main commit M
And   main's CI run for M completed successfully and uploaded
      a crap-baseline artifact
When  the PR's self-score job runs
Then  the baseline is downloaded from M's run — not from whichever
      main run happens to be newest
And   the PR comment reports no NEW/regressed entries originating from
      commits merged before M
```

### Scenario: Race window — main run for M still in progress

```
Given a PR CI run starting seconds after commit M merged to main
And   main's CI run for M is queued or in progress
When  the self-score job resolves the baseline
Then  it waits for M's run, polling with a bounded timeout (5 minutes)
And   if the run completes in time with an artifact, the exact baseline
      is used (no phantom entries)
```

### Scenario: Timeout fallback is explicit, not silent

```
Given the wait above times out (or M's run failed)
When  the self-score job falls back
Then  it walks main's first-parent history from M and uses the nearest
      ancestor commit whose successful run carries a non-expired
      crap-baseline artifact
And   the PR comment carries a visible staleness note naming the
      baseline commit (short SHA) and how many commits behind M it is,
      e.g. "baseline from a7eb656 (1 commit behind) — re-run CI for an
      exact comparison"
```

### Scenario: Docs-only merges fall back without a warning

```
Given main commit M was a docs-only merge, so its CI run succeeded but
      the changes gate skipped self-score (no artifact uploaded)
When  the self-score job resolves the baseline
Then  it does not wait, and walks back to the nearest ancestor with an
      artifact
And   no staleness note is added — a successful run without an artifact
      means the merge changed no analyzed code, so the ancestor baseline
      is score-identical
```

### Scenario: No usable baseline anywhere

```
Given no main commit within the walk depth (30 first-parent commits)
      has a run with a non-expired crap-baseline artifact
When  the self-score job resolves the baseline
Then  the baseline comparison is skipped entirely (existing behaviour)
And   the job still gates on --threshold / --fail-above
```

### Scenario: Push runs on main are unaffected

```
Given a push to main
When  CI runs
Then  the baseline-resolution logic does not run; main uploads its own
      crap-baseline artifact exactly as before
```

---

## Implementation Notes

All changes live in the "Resolve baseline run-id (PRs only)" step of
`ci.yml`; the download and comparison steps are unchanged.

### Resolving MAIN_SHA

On `pull_request` events the checkout is the merge preview, so:

```bash
MAIN_SHA=$(git rev-parse HEAD^1)   # first parent = main tip in the preview
```

(Requires `fetch-depth` deep enough to walk ~30 first-parent commits;
`fetch-depth: 0` on this step's checkout is the simple choice.)

### Lookup by head SHA, not recency

Query runs for the exact commit instead of listing recent runs:

```bash
gh api "repos/$REPO/actions/workflows/ci.yml/runs?branch=main&head_sha=$sha" \
  --jq '.workflow_runs[0]'
```

Walk order: `git rev-list --first-parent -n 30 "$MAIN_SHA"`. For each
SHA, three outcomes matter:

1. run succeeded **and** has a non-expired `crap-baseline` → use it;
   staleness note only if `sha != MAIN_SHA` and the walk passed a
   commit whose run was pending/failed (see 3).
2. run succeeded **without** the artifact (docs-only, changes gate
   skipped self-score) → keep walking, no staleness flag.
3. run queued / in progress (only relevant for `MAIN_SHA` itself) →
   poll up to 5 minutes; on timeout set the staleness flag and keep
   walking. Runs that failed set the flag too.

### Staleness note plumbing

The self-score step already assembles `crap-comment.md` before uploading
it as the `crap-pr-comment` artifact. When the staleness flag is set,
append one line to the file after the generated report:

```
> ⚠ Baseline from `<short-sha>` (<n> commit(s) behind the merge base) —
> re-run CI after main's run completes for an exact comparison.
```

No changes to the cargo-crap binary or the pr-comment renderer — the
note is workflow-level text.

### Non-goals

- No change to the baseline artifact format, name, or retention.
- No change to `pr-comment.yml` (it only relays the artifact).
- No attempt to solve concurrent merges racing each other on main —
  pinning to the merge base makes each PR's comparison self-consistent,
  which is all the gate needs.
