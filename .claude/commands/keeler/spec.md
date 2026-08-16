---
description: Analyze a problem and draft a Gherkin spec for approval
argument-hint: <problem description>
---

Problem to analyze: $ARGUMENTS

You are in the **spec stage** of the workflow (see .claude/keeler.md). Your job is analysis and specification only — **do not write any implementation code, do not create tasks yet**.

1. **Analyze the problem.** Restate it in your own words: who has this problem, what triggers it, what the desired outcome is. List the assumptions you're making and the constraints you see (performance, error handling, edge cases).
2. **Ask clarifying questions** if anything is ambiguous — scope, inputs, failure behavior, non-goals. Prefer asking over assuming for anything that changes the shape of the solution.
3. **Draft the spec.** Copy `specs/TEMPLATE.md` to `specs/NN-<slug>.md` (next free number, kebab-case slug). Fill in:
   - **Context** — the problem analysis from step 1.
   - **Acceptance Tests** — Gherkin scenarios (Given/When/Then) covering the happy path, each edge case, and each failure mode. Scenarios must describe observable behavior, not implementation.
   - **Implementation Notes** — approach sketch, key types, invariants worth a property test, and explicit **Non-goals**.
   - Leave the **Tasks** section empty — that's for /keeler:tasks after approval.
   - Set **Status: Draft**.
4. **Present the spec** to the user: summarize it briefly and point out the decisions you made and the ones you left open.
5. **Stop and wait for approval.** Iterate on feedback until the user approves. On approval, set **Status: Approved**. Only then may /keeler:tasks be run.
