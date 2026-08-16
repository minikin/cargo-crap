---
description: Bugfix pipeline — reproduce with a failing test first, then fix minimally
argument-hint: <bug description or failing behavior>
---

Bug to fix: $ARGUMENTS

You are on the **bugfix path** (see .claude/keeler.md § Change classes). No spec is drafted for a bug — but the discipline is stricter in one way: **no fix may be written before the bug is reproduced by a failing test.**

1. **Understand & reproduce.** Restate the bug: expected vs actual behavior. Write a regression test that fails on the current code — named after the bug's behavior, placed where it belongs (unit test next to the code, or acceptance test if the bug is user-observable). Run it and **show it failing for the right reason**. If you cannot reproduce, stop and report — never "fix" what you can't observe.
2. **Check the spec.** Find the spec covering this behavior:
   - If the bug **contradicts an approved scenario** — it's a plain defect; the scenario is your oracle.
   - If the spec is **silent** (the bug lives in unspecified behavior) — propose a one-scenario spec amendment and wait for approval before fixing; the fix would otherwise encode an undecided requirement.
3. **Fix minimally.** The smallest change that makes the regression test pass without breaking others. Resist drive-by refactoring — if you see unrelated debt, note it in the summary instead.
4. **Gates.** Run `just dev`, then `just mutants-diff`. Surviving mutants on the fixed lines mean the regression test is weaker than it looks — strengthen it (see /keeler:mutants).
5. **Report** (English, per .claude/keeler.md): the reproduction, root cause, the fix, gate results, and anything discovered along the way. Do not commit without confirmation.
