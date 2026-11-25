# crapts

Measure the Change Risk Anti-Patterns (CRAP) score of JavaScript and TypeScript functions using a coverage report.

```sh
npx @minikin/crapts --coverage coverage/lcov.info src
```

Run `npx @minikin/crapts --help` for report formats and threshold options.

Track structural signals against a baseline or use `--duplicates` to find copies
and triage them with TypeSafe or OpenAI. See the
[duplicate guide](https://github.com/minikin/cargo-crap/blob/main/docs/crapts/guides/duplicates.md).

Documentation: https://github.com/minikin/cargo-crap/tree/main/docs/crapts
