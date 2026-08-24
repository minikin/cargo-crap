---
description: Break an approved spec into TDD tasks with acceptance criteria
argument-hint: <spec file, e.g. specs/01-foo.md>
---

Spec to break down: $ARGUMENTS (if empty, use the most recent spec with Status: Approved).

You are in the **task breakdown stage** (see .claude/keeler.md). The spec must have **Status: Approved** — if it's still Draft, stop and tell the user to approve it first (via /keeler:spec iteration).

1. **Read the spec** and its scenarios carefully.
2. **Break the work into ordered tasks** (T1, T2, …), each one small enough to be a single red→green→refactor cycle. For every task specify:
   - a one-line name describing the behavior it delivers,
   - which spec scenarios it covers (every scenario must be owned by exactly one task),
   - which test types pin it: unit, property (state the invariant), and/or acceptance,
   - which earlier tasks it needs, as `Needs: T1, T4.` — omit it for a root, and use the token once per item: the parser counts it, so do not write the word `Needs:` elsewhere in the same item. This is the dependency graph: `scripts/keeler-graph.sh` reads it, and the graph recipes build on it. Two tasks that would edit the **same region** of one file are not independent — give one a `Needs:` on the other, because parallel edits to one region are a merge conflict by construction and the graph is where that is prevented. Files every branch adds to — a recipe in the `Justfile`, a test function, a job in the workflow — merge cleanly if each task adds under its own heading rather than at the end; say where. Hot files with a settled answer: the spec (one line, its own tick), `Cargo.lock` (regenerated on merge), `proptest-regressions/` (append-only seeds), `crap-baseline.json` (never on a branch).
3. **Order tasks** so each leaves the crate compiling and all existing tests green — walking skeleton first, edge cases and failure modes next.
4. **Write the breakdown** into the spec's **Tasks** section as a checkbox list, following the template's format.
5. **Present the plan**: the task list, the scenario→task mapping, and anything you noticed that the spec doesn't cover (propose a spec amendment — never silently extend scope).

Do not start implementing — that's /keeler:tdd, one task at a time.
