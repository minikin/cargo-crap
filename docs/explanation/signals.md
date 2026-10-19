# Structural signals

CRAP says whether a function is complex *and* poorly tested. It does not
say whether the function is hard to read or change, or *why* its score
moved. Structural signals fill that gap with six named, deterministic
counts per function, measured in the same pass as cyclomatic complexity:

| Signal     | What it counts                                                                 |
| ---------- | ------------------------------------------------------------------------------ |
| `nesting`  | Deepest nested bodies: `if`, `match`, loops, `let … else`, closures, `async`.  |
| `lines`    | The function's span, signature to closing brace.                               |
| `bumps`    | Top-level statements whose own nesting reaches 2: separate chunks of nesting. |
| `bool_ops` | Most `&&` / `||` in a single `if` / `while` condition or match guard.          |
| `stubs`    | `todo!`, `unimplemented!`, `dbg!`.                                             |
| `panics`   | `.unwrap()`, `.expect(…)`, `panic!`.                                           |

Turn them on with `[signals] enabled = true` in `.cargo-crap.toml`.

## Regressions, not scores

No threshold for these signals has been calibrated for Rust, so
`cargo-crap` does not invent one. With `--baseline`, it asks the question
`--fail-regression` already asks of CRAP: did this change make this
function worse? A gated signal that rises on an existing function fails
the gate. A function new since the baseline fails on any stub or panic
site (`gate` decides which), and on shape signals only when
`[signals.limits]` sets a ceiling.

Each finding is a *reason*: the signal, its old and new value, the line
span to look at, and a suggested action (`flatten-nesting`,
`split-function`, `extract-condition`, `remove-stub`, `handle-error`,
`add-tests`). A coding agent can fix the function from the JSON alone;
a reviewer sees the same list in the PR comment.

## Why these and not a single number

- No single static metric predicts how well people understand code: of
  121 metrics tested against 63 developers, none correlated significantly
  ([Scalabrino et al., TSE 2019][scalabrino]). Cognitive complexity tracks
  reading time (r≈0.54) but not correctness
  ([Muñoz Barón et al., ESEM 2020][munoz]).
- On expert-labelled code, a plain "over 275 lines" rule (accuracy 0.91)
  beats the Maintainability Index (0.84) ([Borg et al., 2024][ghost]).
- Clippy moved `cognitive_complexity` to `restriction` and points to
  `excessive_nesting` and `too_many_lines` instead
  ([rust-clippy#3793][clippy]).
- Named structural smells (nesting, separate nested chunks, complex
  conditionals, length) are what code-health models weight
  ([CodeScene][codescene], [Bumpy Road][bumpy]).
- Agent-written code drifts: after Cursor adoption, 806 repositories
  showed 30% more static-analysis warnings and 41% more complexity
  ([He et al., MSR 2026][cursor]).
- LLM judges are not fit to gate: they agree with human readability
  ratings at r≈0.25 on Java and ≈0 on Python ([CoReEval][coreeval]).

So the signals stay a vector: no composite score, no invented weights.
Cyclomatic complexity and the CRAP formula are unchanged; CC keeps its
McCabe meaning as the testability term.

[scalabrino]: https://www.cs.wm.edu/~denys/pubs/TSE'19-Understandability.pdf
[munoz]: https://arxiv.org/pdf/2007.12520
[ghost]: https://arxiv.org/html/2408.10754v1
[clippy]: https://github.com/rust-lang/rust-clippy/issues/3793
[codescene]: https://codescene.io/docs/guides/technical/code-health.html
[bumpy]: https://codescene.com/blog/bumpy-road-code-complexity-in-context/
[cursor]: https://arxiv.org/html/2511.04427v3
[coreeval]: https://arxiv.org/html/2510.16579v1

## Limits

- Macro bodies are opaque: control flow inside `tokio::select!` or
  `matches!` is not seen, and neither is a stub or panic site inside
  another macro (`assert_eq!(x.unwrap(), 1)`).
- `.unwrap()` is counted on any receiver; types are not resolved.
- Items nested in a body (a local `fn`) are not counted, like CC.
- A function the delta cannot pair with its baseline record (renamed,
  extracted, or one of several same-named functions that moved) counts as
  new: its shape signals go unchecked unless `[signals.limits]` is set,
  and any stub it already had is reported again.
- Code moved into an item nested in a body (a local `fn`) leaves every
  metric.
- Parallel branches compare against one committed baseline, so a branch
  can inherit a regression another branch merged before it moved.
- `--top` and `--min` narrow the section to the rows shown, while the
  gate still counts every function, as it does for CRAP.
