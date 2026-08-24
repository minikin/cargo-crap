<!-- keeler-version: 0.4.0 -->
# Keeler — workflow rules

The spec-first, test-driven workflow this project follows. Imported by CLAUDE.md.

The stages below are Claude Code slash commands (`.claude/commands/keeler/`)
and the skills beside them; they exist for Claude Code and no other agent.
The gates are plain `cargo` (nextest, llvm-cov, mutants, crap) and `just`,
so anyone — or any tool — can run those. Tests use `proptest` for property
tests.

## Workflow (THE LAW)

Every feature follows this pipeline. Do not skip stages or reorder them.

```
problem ──▶ /keeler:spec ──▶ user approves spec ──▶ /keeler:tasks ──▶ /keeler:tdd (per task)
                ▲                                            │
                └── back-and-forth until approved            ▼
                                                           /keeler:qa  (fmt, lint, tests, coverage, CRAP)
                                                             │
                                                             ▼
                                                          /keeler:review
                                                             │
                                                             ▼
                                                         /keeler:mutants ──▶ survivors? ──▶ strengthen tests,
                                                             │            │          then /keeler:qa + /keeler:mutants again
                                                             ▼            ◀──────────────┘
                                                           done (all gates green)
```

1. **/keeler:spec** — analyze the problem, draft a Gherkin spec in `specs/`, iterate with the user until they approve it. **Never write implementation code at this stage.**
2. **/keeler:tasks** — break the approved spec into ordered tasks, each mapped to its scenarios and test types.
3. **/keeler:tdd** — implement one task at a time, strictly test-first (red → green → refactor). Unit tests + property tests (proptest) + acceptance tests per scenario.
4. **/keeler:qa** — run the full quality gate: `just dev` (fmt, clippy, nextest, doc tests, coverage, cargo-crap).
5. **/keeler:review** — spec-conformance check (scenario→test mapping, scope creep, invariant coverage) done in-project, then the built-in `code-review` skill for generic correctness/simplification — don't hand-roll what it already verifies.
6. **/keeler:mutants** — mutation tests on changed files. Surviving mutants mean the tests are weak: strengthen the tests (never weaken the code to satisfy a mutant), then re-run /keeler:qa and /keeler:mutants until zero survivors.

## Change classes

Not every change is a feature. Pick the lightest class that honestly fits — and never downgrade a change mid-flight to dodge a failing gate:

| Class       | When                                                 | Pipeline                                                                                 |
| ----------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| **Feature** | New behavior, changed behavior, new API              | Full: `/keeler:spec → approve → /keeler:tasks → /keeler:tdd → /keeler:qa → /keeler:review → /keeler:mutants`                       |
| **Bugfix**  | Existing behavior is wrong                           | `/keeler:fix`: failing regression test first → minimal fix → `just dev` + `just mutants-diff`   |
| **Trivial** | Docs, comments, config, renames — no behavior change | Fast path: `just lint` (plus `just test` if code was touched at all); no spec, no review |

Rule of thumb: if you're debating whether it changes behavior, it's not trivial. If a "trivial" change makes any test fail, it wasn't trivial — reclassify.

## Graph mode: the same pipeline, in parallel

The road above is one agent, one branch, stages in sequence. **Graph mode is that same pipeline run by several agents at once** — one per task, each on its own branch in its own worktree, each running `tdd → qa → review → mutants` for the one task it was given. It is opt-in and adds nothing to the linear road: `/keeler:feature` routes exactly as it always did, and a project that never runs the recipes below never notices they are there.

The approved spec is what every spawned agent reads, so the graph lives in the spec and nowhere else. `/keeler:tasks` writes each task's dependencies into the Tasks section as `Needs: T1, T2.` — approving the spec is approving the graph. A task with no `Needs:` is a root, so a spec written before graph mode existed reads as a graph whose every task is ready.

The stages are slash commands; these recipes deliberately are not —
spawning, watching and landing are the human's levers, and the consent
each one grants cannot be given by the agent to itself. When the user asks
in-session ("fan out the spec", "show the board", "land it"), run the
matching recipe below; what you may not do is run `keeler-spawn`,
`keeler-fan-out` or `keeler-land` unasked.

| Command                                       | What it does                                                                                                                                                                                                          |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just keeler-feature-branch <spec>`           | Cuts `feat/<spec-slug>` from main, checks it out and commits the approved spec there — the "graph" answer's one mechanical step, and the same by hand. Reuses the branch when it already exists. |
| `/keeler:graph` — `just keeler-graph <spec>`  | Reads the graph: which tasks are ready, blocked (naming what they wait on) or done. A cycle or a `Needs:` naming no task is refused by the parser, naming the line. Readiness is read from the spec on the feature's own branch, **`feat/<spec-slug>`** — a tick on a task branch unblocks nothing until it lands there. |
| `just keeler-fan-out <spec>`                  | The wave: names every ready, unspawned task and asks once; one **yes** spawns them all through `keeler-spawn`, into one tmux window with a pane per run — nothing starts that was not said yes to. |
| `just keeler-spawn <spec> <task>`             | Cuts the worktree and branch `keeler/<spec-slug>/<task-id>` and hands the task to a headless agent in a detached tmux session, returning at once. Runs only from the feature's branch, **`feat/<spec-slug>`**, and refuses anywhere else. Also refuses a blocked, done or already-spawned task, a spec that differs from HEAD or is not `Approved`, and a machine without tmux. |
| `just keeler-status <spec>`                   | The board: running, passed, `incomplete` (naming which of the three a task still lacks), failed, died mid-pipeline, done, or never spawned — with each run's log and worktree, which are what a resume reads.                                                                             |
| `just keeler-resume <spec> <task>`            | Re-runs a task whose session ended before its gate, in the worktree and branch it already has — the commits on it are how far the pipeline got. Refuses a task still running, one that reached its gate, one that is done, and one never spawned. |
| `just keeler-branch`                          | The gate a task branch runs in place of `just dev`: `dev`, then `crap-delta`, then `mutants-diff` — diff-based by construction.                                                                                        |
| `just keeler-land`                            | Fan-in, at two levels the branch name decides. On the feature branch `feat/<spec-slug>`: `just dev`, then each **closed** task's clean worktree and branch are removed — closed meaning the gate was green, the review record is there and the box is ticked, since a tick alone is the cheapest of the three to produce. On main: `just dev`, then the baseline is regenerated and **staged, never committed**, and a spec whose every box is ticked gets `Status: Implemented` staged beside it. Anywhere else it refuses. |

Three rules keep parallel branches from lying to each other:

- **A branch measures the shared reference; it never moves it.** `crap-baseline.json` and the coverage bar in the `cov` recipe settle at fan-in, on main. CI refuses a `keeler/*` pull request whose diff touched either.
- **A task branch ticks its own task and leaves `Status:` alone.** `Status:` is the one line no task branch may write; `just keeler-land` sets it on main once every box is ticked.
- **Review leaves a record.** /keeler:review writes `reviews/<spec-slug>/<task-id>.md`, and CI on a `keeler/*` pull request fails when it is missing or names a commit the branch did not make.

**A feature, start to finish.** The commands above are the parts; this is the day.

1. `/keeler:spec` on any branch, iterate to approval — as always. On approval it asks which road; **"graph"** is the answer that starts this one, and it runs step 2 for you.
2. `just keeler-feature-branch <spec>` — cuts `feat/<spec-slug>` from main, checks it out and commits the approved spec there, or reuses the branch when it already exists. Answering **graph** is the consent for that one commit, and the question says so before it is answered — as spawning a task is the consent for the commits on its branch. `/keeler:spec` runs this for you on the **graph** answer; run it by hand if you answered linearly and changed your mind. The name is the spec's file name without `.md`, and `keeler-spawn` refuses to run anywhere else.
3. `/keeler:tasks` — writes `Needs:` into every task. Then **commit the graph**: the wave and every spawn read the committed spec, never the working tree.
4. `just keeler-fan-out <spec>` — names every ready task and, on one yes, spawns the wave into a tmux window with a pane per run. `just keeler-graph <spec>` answers the same question without acting on it.
5. Or `just keeler-spawn <spec> T1` one task at a time, when you want to name each. Each returns at once; the agent runs the whole per-task pipeline in its tmux session and ticks its box at the end.
6. `just keeler-status <spec>` for the board; `tmux attach -t keeler-<spec-slug>-t1` to watch, `Ctrl-b d` to leave. `died` means the session ended before its gate ran — its commits are on the branch and its log is under `.keeler/runs/`, which is what a resume starts from.
7. Merge each finished task branch into `feat/<spec-slug>` and run `just keeler-land` there: gates, then the landed worktrees go. The tick has arrived, so its dependents are ready — back to step 5, until `keeler-land` says the feature is finished.
8. Pull request from `feat/<spec-slug>` to main; merge; on main, `just keeler-land` stages the baseline and `Status: Implemented`; you commit.

Three things no step may do for you: an agent never pushes; only `keeler-land` on main writes `Status:`; `keeler-land` runs on the feature branch and on main and refuses everywhere else.

`<spec-slug>` is the spec's file name without `.md`, and the task id is lowercased on the way into every path: `specs/01-login.md` T3 gives the branch `keeler/01-login/t3`, the worktree `../<repo>-01-login-t3`, the tmux session `keeler-01-login-t3` and the record `reviews/01-login/t3.md`. tmux is graph mode's one extra requirement; `just` and the cargo tools are the same ones the linear road uses.

## Commits

**Never commit without explicit user confirmation.** Finish the work, run the gates, then ask — the user decides when a commit happens and may want to review the diff first. This applies to every stage, including "obvious" checkpoints like a finished task or a green pipeline.

**The one place an agent commits without asking is a branch the human spawned.** `just keeler-spawn <spec> <task>` cuts a worktree, creates `keeler/<spec-slug>/<task-id>` in it, and hands it to a headless session that cannot stop to ask. So the word is given in advance and narrowly: on that branch, in that worktree, the agent commits as each stage finishes — which is what gives the review record a `Commit:` to name and leaves the worktree clean for `just keeler-land` to remove. The human named the task and ran the spawn: **that was the asking**, given before rather than after. Nowhere else, and never on main. And never pushed — the branch reaches the remote, and a pull request, only by the human's hand.

## Reporting

**Every finished task ends with a summary in English**, regardless of the conversation language. The summary must cover:

- what was done (tests written, code changed, gates run — with real numbers);
- **problems discovered along the way**: failing gates, surviving mutants, spec gaps, pipeline holes, ambiguities — and how each was resolved or why it was left open;
- open questions that need a user decision (e.g. proposed spec amendments);
- what the next step is;
- a closing one-line status: **PASS** (all gates green), **FAIL** (name the gate that failed), or **FLAKY** (unstable signal — rerun before trusting it).

Never bury a discovered problem in the middle of a transcript — it must reappear in the summary even if it was already mentioned when found.

## Specs (THE LAW)

All feature specs live in `specs/`, written Gherkin style (Given/When/Then). Copy `specs/TEMPLATE.md` for new ones; number them `NN-slug.md`.

- **Never modify a spec file without explicit permission from the user.** The one exception is the `Status:` line and task checkboxes — pipeline bookkeeping: /keeler:spec sets Approved on the user's approval, /keeler:mutants ticks a task's box and sets the spec Implemented when the final gate is clean. Nothing earlier ticks anything — a box ticked after /keeler:tdd would mean "one stage of four ran", and an unreviewed task would look exactly like a finished one.
- A spec's Acceptance Tests section IS the acceptance criteria — every scenario must map to at least one test named after it.
- When a spec needs to change (scope change, new edge case), propose the change and wait for approval before editing the file.

## Quality gates

All of these must be green before a feature is considered done:

| Gate       | Command                                               | Bar                                            |
| ---------- | ----------------------------------------------------- | ---------------------------------------------- |
| Format     | `cargo fmt --all -- --check`                          | clean                                          |
| Lints      | `cargo clippy --all-targets -- -D warnings`           | zero warnings (pedantic is on)                 |
| Tests      | `cargo nextest run --all-targets && cargo test --doc` | all pass                                       |
| Coverage   | `just cov`                                            | no uncovered lines in changed code             |
| CRAP       | `just crap`                                           | no function above threshold 15                 |
| Mutation   | `just mutants-diff`                                   | zero surviving mutants in changed files        |
| CRAP delta | `just crap-delta`                                     | no function's CRAP score regressed vs baseline |

**The review stage leaves no artifact.** Every other gate in this table
fails loudly when it is not met. Review does not — nothing can notice when
it is skipped, and in the repository that ships Keeler it was skipped for
twenty tasks before anyone did. There is no mechanism that will catch this
for you; what there is, is a pipeline whose commands lead from each stage
to the next, and the discipline to follow them.

**Baseline discipline:** `crap-baseline.json` is **committed to the repository** — it is the shared reference every developer and CI measures against, so the ratchet works for the whole team, not just one machine. `just crap-delta` shows per-function before/after and fails on any regression: the "did this change make the codebase worse?" gate. Refresh the baseline (`just crap-baseline`) only deliberately, in its own commit — a moved baseline is a visible decision, reviewable like any other diff.

## Commands

```bash
just            # list recipes
just test       # nextest + doc tests
just lint       # fmt --check + clippy -D warnings
just ci         # lint + test
just cov        # coverage summary (cargo-llvm-cov)
just crap       # coverage + CRAP gate (cargo-crap, threshold 15)
just crap-baseline  # record CRAP baseline before a feature
just crap-delta     # CRAP before/after vs baseline; fails on regression
just dev        # fmt, lint, test, crap — the full fast gate
just mutants src/lib.rs   # mutation tests for one file
just mutants-diff         # mutation tests on files changed vs HEAD
just dev-full   # dev + all mutants (slow)

# Graph mode (opt-in; see the section above)
just keeler-feature-branch specs/01-foo.md  # cut feat/01-foo and commit the approved spec there
just keeler-graph specs/01-foo.md           # ready / blocked / done
just keeler-fan-out specs/01-foo.md         # name every ready task; one yes spawns the wave
just keeler-spawn specs/01-foo.md T3        # hand one ready task to an agent on its own branch
just keeler-status specs/01-foo.md          # what each task is doing right now
just keeler-resume specs/01-foo.md T3       # re-run a task whose session died before its gate
just keeler-branch                          # the gate a task branch runs
just keeler-land                            # fan-in: worktrees on the feature branch, baseline and Status: on main
```

## Skills

Project skills live in `.claude/skills/<name>/SKILL.md` and load automatically when their description matches the work at hand — no invocation needed:

- **property-testing** — invariant catalog and proptest patterns; fires during /keeler:tdd and when a surviving mutant points at a missing law rather than a missing example.
- **gherkin-specs** — how to write observable, testable Given/When/Then scenarios; fires during /keeler:spec and the conformance half of /keeler:review.

Add new skills the same way: a folder under `.claude/skills/`, frontmatter with `name` and a `description` that says *when* to use it — the description is the trigger.

**Recommended user-level skills** for a Rust project on this workflow (installed globally, not shipped with the repo — install your own equivalents if missing):

- **rust-best-practices** — idiomatic Rust: borrowing vs cloning, `Result` error handling, API design; consult while writing or refactoring any Rust code.
- **clean-code** — naming, function size, structure; the go-to during the REFACTOR step of /keeler:tdd.
- **rust-async-patterns** — Tokio, async traits, concurrency; the moment the project grows async code.
- **bulletproof-rust-web** — Axum/Tokio/SQLx/Tower architecture, error handling and production hardening; optional, and only when the project is a web service.

## Testing conventions

- Unit tests live in `#[cfg(test)]` blocks next to the code they test.
- Property tests use `proptest` and live in the same blocks — reach for one whenever the code has an invariant (ordering, idempotence, round-trip, saturation, bounds).
- Acceptance tests live in `tests/acceptance.rs` — one test per spec scenario, named after the scenario, structured as Given/When/Then comments.
- Tests are the spec's enforcement arm: when a mutant survives, the fix is a better test, not a code tweak.
- When proptest finds a counterexample it writes a seed file under `proptest-regressions/` — **commit those files**; they are regression tests that pin the found case forever. (A crate with no `lib.rs` has no anchor for that default path — configure `FileFailurePersistence::WithSource("proptest-regressions")` so the seeds land beside the test file.)
