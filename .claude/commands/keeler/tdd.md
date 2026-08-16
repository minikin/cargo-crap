---
description: Implement one task strictly test-first (red → green → refactor)
argument-hint: <task, e.g. specs/01-foo.md T2; empty = next unchecked task>
---

Task to implement: $ARGUMENTS (if empty, take the first unchecked task from the most recent Approved spec).

You are in the **TDD stage** (see .claude/keeler.md). Implement exactly one task, strictly test-first. Never write production code before a failing test exists.

For the task's each behavior:

1. **RED** — write the test first:
   - unit tests in the module's `#[cfg(test)]` block,
   - a proptest property test for every invariant the task specifies (ordering, idempotence, round-trip, bounds, saturation),
   - acceptance tests in `tests/acceptance.rs` — one per spec scenario, named after the scenario, with Given/When/Then comments.
   Run the test (`cargo nextest run <name>`) and **show that it fails for the right reason** before writing any implementation.
2. **GREEN** — write the minimal implementation that makes the test pass. Resist implementing ahead of the tests.
3. **REFACTOR** — clean up duplication and naming while keeping all tests green. Keep cyclomatic complexity low — anything approaching CRAP threshold 15 gets decomposed now, not later.

After the task's cycles are done:

4. Run `just dev` (fmt, lint, tests, coverage, CRAP). Fix anything red.
5. Report: which tests were written (and the failure you observed at RED), what the implementation does, and gate results.

**Do not tick the task's checkbox.** The box means the whole pipeline ran, and one stage of four has. /keeler:mutants ticks it at the end.

**Next stage: `/keeler:qa` for this task** — not the next task. The pipeline is `tdd → qa → review → mutants`, and a task is finished when it comes out of the far end, not when its tests pass.

If while implementing you discover the spec is wrong or incomplete — stop, propose a spec change, and wait for approval. Never silently drift from the spec.
