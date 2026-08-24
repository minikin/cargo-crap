---
description: Mutation-test changed files; strengthen tests until zero survivors
argument-hint: <file to mutate; empty = files changed vs HEAD>
---

Target: $ARGUMENTS (if empty, run `just mutants-diff` for files changed vs HEAD; with a file argument, `just mutants <file>`).

You are in the **mutation testing stage** (see .claude/keeler.md) — the final gate. A surviving mutant means the test suite would not notice that bug. **The fix is always a stronger test, never a code change to appease the tool** (unless the mutant reveals genuinely dead code — then propose removing it).

1. Run the mutation tests. Read `mutants.out/outcomes.json` / the summary for results.
2. **If zero mutants survived (and none timed out unexpectedly): set the spec's `Status:` to `Implemented`, report the numbers, and close with status PASS.** Mutants are Keeler's control questions — a planted lie the sensors must catch; a survivor means the instrument (the test suite) can't be trusted yet, and the status stays FAIL until it can. **On a `keeler/*` task branch or a `feat/*` feature branch, leave `Status:` exactly as it is** — tick the task's box (step 6) and nothing else: readiness is read from the feature's branch `feat/<spec-slug>`, where this task's tick lands when the branch merges; `just keeler-land` sets `Implemented` on main, at fan-in, once every box is ticked.
3. For each surviving mutant:
   - explain what the mutation was and why no test caught it (missing assertion? untested branch? weak property?),
   - write a test that kills it — test-first: add the test, confirm it passes on real code, and reason about why it would fail on the mutant,
   - prefer strengthening an existing property test's invariant over adding a narrow example test when the survivor points at a general gap.
4. After strengthening tests, re-run the loop as .claude/keeler.md requires:
   - `just dev` (fmt, lint, tests, coverage, CRAP — the new tests changed coverage),
   - the same mutation run again.
5. Repeat until zero survivors. Then report: total mutants, caught/survived per round, which tests were added, and final gate status.
6. **Tick the task's checkbox** in the spec's Tasks section. This is the end of the pipeline, so this is the stage that can honestly say the task is done — tdd → qa → review → mutants all ran.

Then, and only then, move to the next task with `/keeler:tdd`.
