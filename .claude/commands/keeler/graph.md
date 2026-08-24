---
description: Read a spec's task graph — what is ready, blocked, or done
argument-hint: <spec file, e.g. specs/01-foo.md; empty = most recent Approved spec>
---

Spec to read: $ARGUMENTS (if empty, use the most recent spec with Status: Approved).

You are in **graph mode** (see .claude/keeler.md). This command answers one question — which tasks are unblocked right now — and it does not answer it by reading the Tasks section itself. Run:

```
just keeler-graph <spec>
```

and report what it printed, task by task:

- **ready** — every need is done; the task can be spawned or picked up now,
- **blocked** — waiting on the needs the line names; say which,
- **done** — its box is ticked.

Say plainly when nothing is ready: either the graph is complete (every task done) or every unfinished task is blocked, and those are different states.

If the recipe **refuses** — a cycle, a Needs: naming no task, an id defined twice, two Needs: in one item — it exits non-zero naming the line and what is wrong. Report the refusal verbatim and stop: nothing is ready until the Tasks section is fixed, and that is a spec edit — /keeler:tasks, with the user's say-so — not something to patch from here. The refusal is the script's, on purpose: a cycle is refused by a program, so the verdict is machine-checkable and not an agent's opinion. Do not second-guess it by reading the spec.

Readiness is read from the spec on the feature's own branch, **`feat/<spec-slug>`** — which is also the only branch `just keeler-spawn` will run from. A tick on a task branch unblocks nothing until it lands on the feature branch, because arriving there is the landing.

Do not implement anything here — that is /keeler:tdd on a ready task, or `just keeler-spawn <spec> <task>` to hand one to an agent on its own branch.
