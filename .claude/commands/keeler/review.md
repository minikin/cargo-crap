---
description: Review the change — spec conformance ourselves, then the built-in code-review
argument-hint: <spec file; empty = most recent Implemented/Approved spec>
---

Spec under review: $ARGUMENTS (if empty, use the spec whose tasks were just implemented).

You are in the **review stage** (see .claude/keeler.md). The review has two parts: a spec-conformance pass that only this project can do, then the built-in code-review skill for everything generic.

## Part 1 — Spec conformance (do this yourself)

Check the diff (`git diff HEAD`, or the last commit if the tree is clean) against the approved spec:

1. Every spec scenario has at least one test **named after it** (acceptance tests in `tests/acceptance.rs`, properties in the module).
2. No behavior exists that no scenario demands (scope creep) — flag it as a proposed spec amendment, never silently keep it.
3. Every invariant listed in the spec's Implementation Notes is pinned by a property test.
4. Error messages / public API match what the scenarios promise (tokens named verbatim, etc.).

## Part 2 — Generic review (delegate)

Invoke the built-in `code-review` skill on the current diff. It covers correctness bugs, simplification, reuse, and efficiency with adversarial verification of findings — do not duplicate that analysis by hand. Use a higher effort level when the change is large or risky.

## Report

Merge both parts into one ranked findings list (spec-conformance findings first — a conformance gap outranks a style cleanup). For each finding: file:line, what's wrong, concrete failure scenario, suggested fix.

**Do not apply fixes** — the user decides what to act on. Agreed fixes go back through /keeler:tdd (test first), then /keeler:qa, and return here.

## The record

Review is the one stage that would otherwise leave no evidence, so it leaves some. When the findings are settled — on the last pass through this stage, not the first — write `reviews/<spec-slug>/<task-id>.md`, where `<spec-slug>` is the spec's file name without `.md` (`01-login` for `specs/01-login.md`) and `<task-id>` is the task id in lowercase (`t3` for T3). It opens with exactly four header lines, then the findings, or the word `none` — a review that found nothing still happened:

```
Spec: <spec-slug>
Task: <task-id>
Commit: <the SHA this review examined>
Verdict: pass | fail
```

`Commit:` is the SHA the review examined — `git rev-parse HEAD` on a clean tree, or the last commit of the diff you read. On a keeler/* task branch that is a commit the branch itself made, and CI on the pull request checks exactly that: the record exists, and its `Commit:` is an ancestor of the branch head and not of the base branch. The record is committed like the rest of the stage's work.

**Next stage: `/keeler:mutants`**, once the findings are settled.
