# Shields.io badge

`--format shields` emits a single JSON object following the
[Shields.io endpoint schema](https://shields.io/badges/endpoint-badge).
Serve the file at a stable URL (GitHub Pages, raw blob) and embed it as a
normal badge image:

```markdown
![CRAP](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/owner/repo/main/crap-badge.json)
```

The label embeds the *effective* threshold: `CRAP > 30` by default, or
whatever `--threshold` was given (`CRAP > 15` in this repo's own run), so the
badge reads as a complete statement. The message is `passing` (brightgreen)
when no function exceeds `--threshold`, `N crappy` in yellow for 1–5
offenders, and red for 6 or more. `--baseline` is silently ignored, since the
badge always reflects absolute current scores. See [Badge generation](#badge-generation)
for a CI recipe.

## Badge generation

Regenerate the badge JSON on every push to the default branch and commit
it back so the README embed stays current:

```yaml
- name: Generate CRAP badge
  run: |
    cargo crap \
      --lcov lcov.info \
      --workspace \
      --threshold 30 \
      --format shields \
      --output crap-badge.json

- name: Commit badge
  run: |
    git config user.name "github-actions[bot]"
    git config user.email "github-actions[bot]@users.noreply.github.com"
    git add crap-badge.json
    git diff --cached --quiet || git commit -m "chore: update CRAP badge"
    git push
```

The badge at the top of the README comes from a different shape: `ci.yml`
uploads `crap-badge.json` as an artifact and a separate `badge` job pushes it
to a dedicated `badges` branch, so the default branch never carries a
generated file. See [`.github/workflows/ci.yml`](https://github.com/minikin/cargo-crap/blob/main/.github/workflows/ci.yml) if
you want that instead.
