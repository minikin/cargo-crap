# Spec 32: A provider seam for duplicate-pair triage

**Status:** Approved
**Effort:** Large
**Module:** `src/duplicates/triage/` (new `provider/` with `typesafe.rs` and `openai.rs`, plus `client.rs`, `request.rs`, `verdict.rs`, `cache.rs`, `mod.rs`), `src/config.rs`, `src/main.rs`, `docs/guides/triage.md`, `docs/reference/config.md`, `README.md`

## Context

Spec 30 asks a model three typed questions about each duplicate pair and
prints the answers beside it. It was written against one API, TypeSafe's
`/v1/systemone`, and the code says so everywhere: the endpoint path, the
`TYPESAFE_API_KEY` and `TYPESAFE_BASE_URL` variables, the wire shape of the
questions, the response decoder and the error text are all TypeSafe's.

A second API now answers the same kind of question. OpenAI's Decisions API
went to public beta on 2026-10-06: `POST /v1/decisions` on `gpt-6-luna`
returns typed answers (predicates, choices with a confidence, scores over
ordered levels) instead of prose. Its three answer types line up with the
three triage questions one for one. A team that already pays for OpenAI, or
that cannot send code to a second vendor, has no way to use triage today.

The goal is not "add OpenAI". It is **a seam**: triage asks a provider-neutral
question set, and a provider is one file that translates it to a wire shape,
names its endpoint and its environment, and decodes the answers back. TypeSafe
becomes the first implementation of that seam and OpenAI the second. A third
provider later is one new file and one registry entry.

The confidence floor is applied as it is to whatever confidence the provider
returns. Each provider calibrates its own confidence, so the same floor can
mark a different share of pairs `uncertain` under each one.

**Nothing changes for a project that does not opt in.** TypeSafe stays the
default, its requests stay byte-identical, its cached verdicts stay valid, and
the human and JSON output do not change shape.

### What the OpenAI wire format is based on

The OpenAI guide at
`https://developers.openai.com/api/docs/guides/decisions`, read on
2026-10-09. The guide gives the request fields and three response examples.
It documents no limits, no error body format and no refusal object beyond
`type` and `name`, and the API reference page was not reachable. A live
check (see Testing) is what confirms the decoder against the real service.

Request:

```json
{
  "model": "gpt-6-luna",
  "input": "<text>",
  "questions": [
    {"type": "choice", "name": "...", "instructions": "...",
     "choices": [{"value": "...", "description": "..."}]},
    {"type": "score", "name": "...", "instructions": "...",
     "levels": [{"label": "...", "description": "..."}]},
    {"type": "predicate", "name": "...", "instructions": "..."}
  ]
}
```

Response: `{"answers": [...]}`, one entry per question, identified by the
echoed `name`. A choice answer carries `choice`, `probabilities` and
`confidence`. A score answer carries `score` (a probability-weighted level
index, lowest level 0), `probabilities` and `confidence`. A predicate answer
carries `probability`. An answer of `"type": "refusal"` declines the question.

### Constraints

Every constraint of spec 30 holds for every provider:

- **The gate must not move.** No provider influences the exit code,
  `crap-delta` or the baseline.
- **The API key is never configuration.** Each provider reads its key from
  its own environment variable and from nowhere else.
- **A key belongs to its provider.** A key set for one provider is never sent
  to another.
- **Offline keeps working.** No key, no network or a failing API gives the
  spec-29 output and the same exit code.
- **All or nothing.** One failed pair discards the run's triage.
- **One build-time opt-in.** The `triage` Cargo feature compiles the client
  for every provider. There is no per-provider feature.

### Rejected alternatives

- **A CLI flag to pick the provider.** Rejected for the reason spec 30 gave:
  sending code to a vendor is a project decision. Configuration only.
- **An OpenAI-compatible "base URL" switch on the TypeSafe client.** Rejected:
  the two APIs differ in request shape, response shape and answer types, not
  just in host. A URL switch would send TypeSafe's body to OpenAI.
- **Per-provider question wording.** Rejected: the cached verdicts, the
  rendering and the floor all assume the same three questions. Providers
  translate one question set and never rewrite it.
- **A refusal as a new output state.** Rejected: a refusal is not a judgment,
  and a new line shape widens the human and JSON contract for one provider's
  failure mode. It fails the pair, like any other answer that cannot be used.
- **Re-keying the whole cache by provider.** Rejected: every project with a
  warm TypeSafe cache would pay for every pair again after upgrading.

---

## Acceptance Tests

### Scenario: TypeSafe stays the default provider

```
Given a .cargo-crap.toml whose [duplicates.triage] table enables triage
      and names no provider
And   TYPESAFE_API_KEY is set
When  cargo-crap runs with duplicate detection
Then  every request goes to the TypeSafe endpoint /v1/systemone
And   each request body is byte-identical to the golden body recorded from
      the same pair before the seam was introduced
And   the warnings a missing key or a failing API print are unchanged
```

### Scenario: Choosing OpenAI sends each pair to the Decisions endpoint

```
Given triage is enabled with provider = "openai" and no model configured
And   OPENAI_API_KEY is set
And   duplicate detection reports two pairs
When  cargo-crap runs
Then  two requests go to /v1/decisions
And   each carries the header "Authorization: Bearer <OPENAI_API_KEY>"
And   each names the model gpt-6-luna
And   each pair is followed by a triage line
```

### Scenario: An OpenAI request carries exactly the pair under judgment

```
Given triage is enabled with provider = "openai" and two pairs were found
When  cargo-crap runs against a recording API
Then  each request's input contains the two function bodies of one pair
      and their locations
And   no request contains a function body from any other pair
And   each request asks three questions: a choice offering the four kinds,
      a score over the four worth-extracting levels from lowest to highest,
      and a predicate for divergence risk
```

### Scenario: A configured model overrides the provider's default

```
Given triage is enabled with provider = "openai" and model = "stub-model"
When  cargo-crap runs
Then  every request names the model stub-model
```

### Scenario: The same judgment prints the same whichever provider made it

```
Given a project where duplicate detection reports two pairs
And   a TypeSafe stub and an OpenAI stub that each answer every pair with
      the same kind, worth-extracting score, divergence probability and
      confidence, in their own wire shape
When  cargo-crap runs once against each, in human and in json
Then  the two human outputs are byte-identical
And   the two json outputs are byte-identical
```

### Scenario: A missing OpenAI key degrades to the untriaged report

```
Given triage is enabled with provider = "openai"
And   OPENAI_API_KEY is unset
When  cargo-crap runs
Then  the duplicates section is byte-identical to the spec-29 output
And   stderr carries a warning naming OPENAI_API_KEY
And   the exit code is what the same run would produce with triage disabled
And   no network request is made
```

### Scenario: A key set for another provider is never used

```
Given triage is enabled with provider = "openai"
And   TYPESAFE_API_KEY is set and OPENAI_API_KEY is unset
When  cargo-crap runs
Then  no network request is made
And   stderr carries a warning naming OPENAI_API_KEY
```

### Scenario: An unreachable OpenAI API degrades to the untriaged report

```
Given triage is enabled with provider = "openai" and the key is set
And   every request to the API fails
When  cargo-crap runs
Then  the duplicates section is byte-identical to the spec-29 output
And   stderr carries a warning naming the OpenAI API and the failure
And   the exit code is what the same run would produce with triage disabled
```

### Scenario: A refused question discards the whole triage

```
Given triage is enabled with provider = "openai" and three pairs were found
And   the API refuses one question about one pair
When  cargo-crap runs
Then  no pair carries a triage line
And   stderr carries a warning that says the question was refused and
      names it
And   the exit code is what the same run would produce with triage disabled
```

### Scenario: An OpenAI answer that cannot be decoded degrades

```
Given triage is enabled with provider = "openai"
And   the API answers the kind question with a choice that was not offered
When  cargo-crap runs
Then  no pair carries a triage line
And   stderr carries a warning naming the question and the unexpected value
```

### Scenario: OPENAI_BASE_URL redirects the OpenAI requests

```
Given triage is enabled with provider = "openai" and the key is set
And   OPENAI_BASE_URL is "http://127.0.0.1:<port>/v1"
When  cargo-crap runs
Then  every request goes to http://127.0.0.1:<port>/v1/decisions
```

### Scenario: An unknown provider is rejected before any analysis

```
Given a .cargo-crap.toml with provider = "acme" in [duplicates.triage]
When  cargo-crap runs
Then  it exits with the configuration-error code
And   the message names duplicates.triage.provider and the accepted values
And   no analysis and no network request happen
And   the same holds for a build without the `triage` feature
```

### Scenario: A build without the triage feature accepts any known provider

```
Given a cargo-crap built without the `triage` feature
And   a .cargo-crap.toml that enables triage with provider = "openai"
When  cargo-crap runs with duplicate detection
Then  the duplicates section is byte-identical to the spec-29 output
And   stderr carries one warning naming the `triage` feature
And   no network request is made
```

### Scenario: Existing TypeSafe verdicts stay cached across the upgrade

```
Given a verdict cache entry stored under the key spec 30 computes for a
      pair's two bodies, the model jev-latest and the current question set
And   neither body has changed
When  cargo-crap runs with triage enabled and no provider named
Then  the pair carries the cached triage line
And   no network request is made
```

### Scenario: Switching provider never reuses the other provider's verdicts

```
Given a TypeSafe run cached a verdict for every pair under model name M
When  the provider is switched to "openai" with model = M
And   cargo-crap runs
Then  a request is made for every pair
```

### Scenario: A second OpenAI run over unchanged code asks nothing

```
Given an OpenAI run cached its verdicts
And   no function body in any pair has changed
When  cargo-crap runs again with the same provider and model
Then  every pair carries the same triage line as the previous run
And   no network request is made
```

---

## Tasks

Acceptance tests for this spec go in `tests/acceptance.rs` under a
`Spec 32` section that T1 lays out, with one heading per owning task
(`// ---- Spec 32 · T1 ----`, `· T5`, `· T6`, `· T7`, `· T8`, `· T9`). Each
task fills only its own heading.

- [x] **T1 — Pin the TypeSafe request before the seam, and rename the stub.** On current code, record the TypeSafe request body for one fixed pair into `tests/fixtures/triage/golden/typesafe-request.json` and add an acceptance test that runs the binary with no provider named and compares every recorded body to it, checks the path is `/v1/systemone`, and checks the missing-key and failing-API warnings are unchanged. Rename `tests/support/typesafe_stub.rs` to `api_stub.rs` (its `base_url()` doc stops naming `TYPESAFE_BASE_URL`) and lay out the `Spec 32` section with its per-task headings. Scenarios: _TypeSafe stays the default provider_. Tests: acceptance.
- [x] **T2 — The seam, with TypeSafe as its first provider.** Needs: T1. `src/duplicates/triage/provider/mod.rs` (the `Provider` trait, the static registry, `QuestionSet`, `Answers`) and `provider/typesafe.rs`, compiled in every build. `request::questions` builds from `QuestionSet`. The TypeSafe decoder moves into `typesafe.rs` and returns `Answers`, and range checks with the `TOLERANCE` clamp move to one `Answers` to `Verdict` step in `verdict.rs`. `DEFAULT_TRIAGE_MODEL` becomes `Provider::default_model()`. Scenarios: none end-to-end (T1's test stays green). Tests: unit + property (TypeSafe wire identity: for any pair, the body built through the seam equals the spec-30 builder, kept as a test oracle. Encode and decode agree: every question encoded has an answer decoded by the same name. Registry ids are unique).
- [x] **T3 — The OpenAI provider.** Needs: T2. `provider/openai.rs` plus its registry entry. State serialized into `input`, questions as an array with `choices`, `levels` and `predicate` (binary rubrics appended to the instructions), answers matched by `name`, a refusal decoded as an error that says the question was refused, a choice not offered rejected naming the question, the lowest offered level mapped to 0. Scenarios: none end-to-end. Tests: unit (decode the guide's three example shapes, refusal, unknown choice, missing answer) + property (for any `Answers`, the TypeSafe and the OpenAI wire responses carrying it decode to the same `Verdict`. Encode and decode agree).
- [x] **T4 — Provider-aware settings and errors.** Needs: T2. `client.rs`: `Settings::from_env` and `from_lookup` take the provider and read only its key and base-URL variables, `endpoint()` joins the provider's path, `TriageError` names the provider's display name and key variable, with TypeSafe's messages unchanged byte for byte. Scenarios: none end-to-end. Tests: unit (the other provider's variables are ignored, the OpenAI endpoint is base plus `/decisions`, each error names its provider) + property (for any lookup, settings for one provider never carry a value read from another provider's variables).
- [x] **T5 — The `provider` config key and its validation.** Needs: T3. `TriageConfig` gains `provider` in `src/config.rs`, and `validate_merged_values` in `src/main.rs` rejects an id the registry does not hold, naming `duplicates.triage.provider` and the registered ids, in every build. Scenarios: _An unknown provider is rejected before any analysis_, _A build without the triage feature accepts any known provider_. Tests: unit (parse, alias, default) + property (every registered id validates, and the error lists exactly the registered ids) + acceptance.
- [x] **T6 — Wire the provider into the run.** Needs: T4, T5. `DupSettings` in `src/main.rs` resolves the provider, then the model (an unset model follows the provider), and passes the provider to `Settings::from_env`. `triage::run` encodes and decodes through it. Scenarios: _Choosing OpenAI sends each pair to the Decisions endpoint_, _An OpenAI request carries exactly the pair under judgment_, _A configured model overrides the provider's default_, _OPENAI_BASE_URL redirects the OpenAI requests_, _A missing OpenAI key degrades to the untriaged report_, _A key set for another provider is never used_. Tests: acceptance, against the stub.
- [x] **T7 — OpenAI failures degrade like TypeSafe's.** Needs: T6. Scenarios: _An unreachable OpenAI API degrades to the untriaged report_, _A refused question discards the whole triage_, _An OpenAI answer that cannot be decoded degrades_. Tests: acceptance + property (degradation identity: for any failure the stub injects under `provider = "openai"`, stdout equals the same run with triage disabled, and the exit code matches).
- [x] **T8 — Namespace the cache by provider.** Needs: T6. `CacheKey::new` in `cache.rs` takes the provider: TypeSafe's key stays exactly spec 30's, every other provider also hashes its length-prefixed id. `Run::judge` in `triage/mod.rs` passes it. Scenarios: _Existing TypeSafe verdicts stay cached across the upgrade_, _Switching provider never reuses the other provider's verdicts_, _A second OpenAI run over unchanged code asks nothing_. Tests: unit (the existing key-literal pin still holds) + property (for any bodies and model, the OpenAI key differs from the TypeSafe key, and the TypeSafe key equals spec 30's) + acceptance.
- [x] **T9 — The same judgment prints the same under either provider.** Needs: T6. Scenarios: _The same judgment prints the same whichever provider made it_. Tests: acceptance (one run per provider against stubs answering the same judgment in each wire shape, human and json compared byte for byte).
- [x] **T10 — Live OpenAI check.** Needs: T6. `#[ignore]` tests in `tests/triage_live.rs`, beside the TypeSafe ones, on the two fixtures in `tests/fixtures/triage/`: the same-logic pair must come back `same-logic`, the idiom-only pair must not be marked for merging and must decode to a worth-extracting level near the bottom. The `triage-live` recipe in the `Justfile` takes the provider and selects the matching tests by name. Scenarios: none (confirms the wire format against the real service). Tests: ignored live tests.
- [ ] **T11 — Documentation.** Needs: T6. `docs/guides/triage.md`, `docs/reference/config.md` and `README.md` gain the `provider` key, the per-provider environment table and the data-leaves-the-machine note for each provider. The triage object's description in `schemas/report-v1.json` and `schemas/delta-v2.json` stops saying TypeSafe. `examples/triage-demo` is checked for TypeSafe-only wording, and `CHANGELOG.md` gets an Unreleased entry. Scenarios: none. Tests: the existing docs-drift and schema tests stay green.

---

## Implementation Notes

### Data flow

```
Vec<DuplicatePair> ──▶ request: re-read both bodies (unchanged)
                          │
                          ▼
                 QuestionSet (neutral, one definition)
                          │
          ┌───────────────┴───────────────┐
          ▼                               ▼
 provider::TypeSafe              provider::OpenAi
 encode ──▶ /v1/systemone body   encode ──▶ /v1/decisions body
 decode ◀── answers map          decode ◀── answers array
          └───────────────┬───────────────┘
                          ▼
              Answers (neutral) ──▶ Verdict ──▶ Assessment (unchanged)
```

The HTTP client (retries, back-off, `Retry-After`, timeouts) is shared. It is
handed an endpoint and a key by the provider and never sees a wire shape.

### The seam

```rust
pub trait Provider: Sync {
    /// The config value and cache namespace: "typesafe", "openai".
    fn id(&self) -> &'static str;
    /// The name errors and warnings use: "TypeSafe API", "OpenAI API".
    fn display_name(&self) -> &'static str;
    fn key_var(&self) -> &'static str;
    fn base_url_var(&self) -> &'static str;
    fn default_base_url(&self) -> &'static str;
    fn endpoint_path(&self) -> &'static str;
    fn default_model(&self) -> &'static str;
    fn encode(&self, state: &Value, model: &str, questions: &QuestionSet) -> Value;
    fn decode(&self, body: &str) -> Result<Answers, DecodeError>;
}
```

A static registry maps `id` to `&'static dyn Provider`. The trait, the
registry and both implementations compile in every build, like `request` and
`verdict`: config validation lists the registered ids, and that runs without
the `triage` feature too. Only `client` and `triage::run` stay behind it.

Every provider authenticates with `Authorization: Bearer <key>` and sends
`Content-Type: application/json`, which is all the shared client sends today.
The trait has no header hook. A provider that needs more headers adds one,
and the live check is what confirms the Decisions API needs none.

The default model moves from `DEFAULT_TRIAGE_MODEL` in `src/config.rs` to
`Provider::default_model()`. `DupSettings` in `src/main.rs` resolves the
provider first and then the model, so an unset `model` follows the provider.
`Settings::from_env` and `from_lookup` in `client.rs` take the provider and
read only its key and base-URL variables, and `Settings::endpoint` joins the
provider's path. The acceptance bar
for "any provider later" is that a third one needs **one new file implementing
the trait and one registry entry**, and nothing else: no change to the
client, the cache, the rendering or the config parser, which accepts whatever
ids the registry holds and names them in its error.

`QuestionSet` holds the three questions as data, each with an id, its
instructions and its type:

- choice: a value and a rubric per kind
- score: a label and a rubric per level, lowest first
- binary: the rubric for true and the rubric for false

It replaces the JSON literals in `request::questions`. TypeSafe's `encode` must
produce exactly today's body, which the default-provider scenario pins.

A provider's `decode` maps the chosen value back to a `Kind` and rejects a
value that was not offered, naming the question. It also maps the score so
that the lowest offered level is 0, whatever index the wire uses.

`Answers` holds what `Verdict` needs: the chosen kind and its confidence, the
score and its confidence, and the divergence probability. The binary answer
carries a probability and no confidence, because neither API returns one for
it. Range checks and the
`TOLERANCE` clamp move from the TypeSafe decoder into the one place that
turns `Answers` into a `Verdict`, so every provider gets the same checks.

### Wire mapping

| Neutral | TypeSafe | OpenAI |
| --- | --- | --- |
| state | `state` object | `input`: the state serialized as JSON text |
| questions | `questions` object keyed by id | `questions` array, id in `name` |
| choice options | `criteria` map, value to rubric | `choices: [{value, description}]` |
| score levels | `criteria` array of rubrics | `levels: [{label, description}]` |
| binary | `noul` with `true`/`false` criteria | `predicate`, with the two rubrics appended to `instructions` |
| answers | `answers` object keyed by id | `answers` array, matched by `name` |
| binary answer | `noul` | `probability` |
| refusal | none | `"type": "refusal"`: a decode error naming the question |

A predicate carries no criteria field, so the OpenAI encoder appends the
true and false rubrics to the instructions text. That is the same data in the
only slot the wire shape offers, not a rewording.

The instructions already refer to `function_a`, `function_b` and
`structural_similarity`, which are the keys of the serialized state, so they
read the same in both shapes.

### Environment

| Provider | Key | Base URL variable | Default base URL | Path |
| --- | --- | --- | --- | --- |
| `typesafe` | `TYPESAFE_API_KEY` | `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` | `/v1/systemone` |
| `openai` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | `/decisions` |

`OPENAI_BASE_URL` follows the OpenAI SDK convention, so it includes `/v1`
(the Python SDK defaults it to `https://api.openai.com/v1`). A
user who already set it for a proxy or a gateway gets the same host here.
Only the selected provider's variables are read.

### Configuration

```toml
[duplicates.triage]
enabled = true
provider = "openai"       # default "typesafe"
model = "gpt-6-luna"      # default: the provider's default model
confidence-floor = 0.5
```

`provider` is validated beside the confidence floor in
`validate_merged_values`, with the same error shape and exit code.

### Cache

The key is FNV-1a over both bodies, the model and `QUESTION_SET_VERSION`.
For TypeSafe it stays exactly that, so existing entries keep matching. Every
other provider also hashes its `id`, length-prefixed like the other fields,
so two providers that share a model name (through a gateway, say) never share
a verdict. `QUESTION_SET_VERSION` does not change: the questions do not.

### Errors

`TriageError` names the provider: `MissingKey` carries the variable, and
`Status`, `Transport` and `Decode` carry the display name, replacing the
hard-coded "TypeSafe API". A refusal is a `DecodeError` naming the refused
question, so it takes the existing all-or-nothing path.

### Invariants worth a property test

- **TypeSafe wire identity.** For any pair, the TypeSafe body built through
  the seam equals the body spec 30 built.
- **Rendering is provider-neutral.** For any `Answers`, both providers' wire
  responses carrying it decode to the same `Verdict`.
- **Encode and decode agree.** For each provider, every question `encode`
  sends has an answer `decode` reads by the same name, and every choice value
  and level it offers is one `decode` accepts.
- **Cache keys are namespaced.** For any bodies and model, the OpenAI key
  differs from the TypeSafe key, and the TypeSafe key equals spec 30's.
- **Registry completeness.** Every registered id parses as a config value,
  and the config error lists exactly the registered ids.

### Testing

Before the seam is cut, a test on current main records the TypeSafe request
body for a fixed pair into a committed golden file. The default-provider
scenario compares against that file, so the identity claim has a red state.

The recording stub (`tests/support/typesafe_stub.rs`) already serves scripted
JSON and records paths, headers and bodies, so it serves OpenAI-shaped
answers unchanged. It is renamed to `api_stub.rs` because it is no longer
TypeSafe's, and its `base_url()` doc stops naming `TYPESAFE_BASE_URL`. An
OpenAI test sets `OPENAI_BASE_URL` to the stub's URL plus `/v1`. The OpenAI
`input` is JSON inside a string, so a test that checks the questions or the
state parses the body and then parses `input`. No test may need a network or
a key.

The tests that assert the exact missing-key message
(`tests/acceptance.rs`, `tests/triage_client.rs`, `client.rs`) keep passing
unchanged: the TypeSafe message is part of the default-provider identity.

A live `#[ignore]` check mirrors spec 30's T11 for OpenAI on the same two
fixtures in `tests/fixtures/triage/`, in `tests/triage_live.rs` beside the
TypeSafe one. The `triage-live` recipe takes the provider as a parameter and
selects the matching tests by name. The live OpenAI check also asserts that
the idiom-only fixture decodes to a worth-extracting level near the bottom,
which confirms the score indexing. It is the only check of the decoder against the real
service, since the guide this spec is based on documents no refusal fields
and no error body.

### Documentation

`docs/guides/triage.md`, `docs/reference/config.md` and the README name
TypeSafe as the only provider. They gain the `provider` key, the
per-provider environment table and a note that each provider receives the two
function bodies, as spec 30 says for TypeSafe. `CHANGELOG.md` gets an
Unreleased entry. The triage object's description in `schemas/report-v1.json`
and `schemas/delta-v2.json` stops saying TypeSafe (its shape does not change),
and `examples/triage-demo` is checked for TypeSafe-only wording.

### Non-goals

- **A provider option in the output.** The human line and the JSON triage
  object do not name the provider or the model.
- **Several providers in one run**, fallback from one to another, or
  comparing their answers.
- **Image input.** The Decisions API accepts images. Triage sends code.
- **Providers beyond TypeSafe and OpenAI**, a local model, or a plug-in
  mechanism outside the crate. The seam makes a third provider cheap. It does
  not load one at run time.
- **Per-provider Cargo features.**
- **Validating model names.** Model ids change faster than releases. An
  unknown model is the API's to refuse, and the refusal degrades like any
  other failure.
