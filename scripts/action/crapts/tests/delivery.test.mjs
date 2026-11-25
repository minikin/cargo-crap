import assert from "node:assert/strict";
import { test } from "node:test";
import { firstLineMatches, graphqlUrl, jobSummary, ownedBy } from "../delivery.mjs";
import { scanOnly, trustedUrls, unpaidStderr, withTrustedUrls, withoutOptions, withoutProviderKeys } from "../cli.mjs";

test("only a complete first marker line matches", () => {
  let state = 14;
  for (let i = 0; i < 500; i++) {
    let marker = "";
    for (let j = 0; j <= i % 45; j++) {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      const point = state % 0x110000;
      marker += [10, 13].includes(point) ? " " : String.fromCodePoint(point);
    }
    for (const body of [marker, marker + "\nbody", marker + "\r\nbody"]) assert.ok(firstLineMatches(body, marker));
    for (const body of [marker + "x\n", "quoted\n" + marker]) assert.ok(!firstLineMatches(body, marker));
  }
});

test("summary byte budget includes settings and markup", () => {
  const settings = '{"threshold":30}\n';
  const unit = "\u00e9";
  const overhead = Buffer.byteLength(jobSummary("", settings));
  for (const length of [4096, 999999, 1000000]) {
    const available = length - overhead;
    const report = unit.repeat(Math.floor(available / 2)) + "x".repeat(available % 2);
    const summary = jobSummary(report, settings);
    if (length < 1000000) {
      assert.equal(Buffer.byteLength(summary), length);
      assert.ok(summary.includes(report) && summary.includes(settings));
    } else {
      assert.equal(summary.trim().split("\n").length, 1);
      assert.match(summary, /report.*output/);
    }
  }
});

test("a comment belongs to the token's account by login, a bot also without its suffix", () => {
  const bot = { login: "github-actions[bot]", type: "Bot" };
  assert.ok(ownedBy({ user: bot }, "github-actions[bot]"));
  assert.ok(ownedBy({ user: bot }, "github-actions"));
  assert.ok(ownedBy({ user: { login: "octo", type: "User" } }, "octo"));
  assert.ok(!ownedBy({ user: { login: "renovate[bot]", type: "Bot" } }, "github-actions"));
  assert.ok(!ownedBy({ user: { login: "octo[bot]", type: "User" } }, "octo"));
  for (const comment of [{}, { user: null }]) assert.ok(!ownedBy(comment, "octo"));
  for (const login of ["", null, undefined]) assert.ok(!ownedBy({ user: { login: "", type: "User" } }, login));
});

test("GraphQL lives beside REST, at /api/graphql on Enterprise Server", () => {
  assert.equal(graphqlUrl("https://api.github.com"), "https://api.github.com/graphql");
  assert.equal(graphqlUrl("https://ghe.example/api/v3"), "https://ghe.example/api/graphql");
  assert.equal(graphqlUrl("https://ghe.example/api/v3/"), "https://ghe.example/api/graphql");
  assert.equal(graphqlUrl("http://127.0.0.1:8765"), "http://127.0.0.1:8765/graphql");
});

test("provider keys are removed whatever their case", () => {
  const env = { PATH: "/bin", typesAfe_api_key: "a", OpenAI_Api_Key: "b", TYPESAFE_API_KEY: "c", TYPESAFE_BASE_URL: "u" };
  assert.deepEqual(withoutProviderKeys(env), { PATH: "/bin", TYPESAFE_BASE_URL: "u" });
});

test("the paid run uses only the provider URLs recorded at install", () => {
  const env = { PATH: "/bin", TYPESAFE_API_KEY: "k", typesafe_base_url: "http://late", OPENAI_BASE_URL: "http://late" };
  assert.deepEqual(withTrustedUrls(env, { TYPESAFE_BASE_URL: "http://early" }),
    { PATH: "/bin", TYPESAFE_API_KEY: "k", TYPESAFE_BASE_URL: "http://early" });
  assert.deepEqual(withTrustedUrls(env, {}), { PATH: "/bin", TYPESAFE_API_KEY: "k" });
  assert.deepEqual(trustedUrls({ openai_base_url: "http://o", PATH: "/bin" }), { OPENAI_BASE_URL: "http://o" });
});

test("only the unset-key triage warning is dropped from an unpaid run", () => {
  const kept = "warning: No coverage supplied. Scores use the pessimistic missing policy.\n" +
    "warning: duplicate triage skipped: HTTP 500; the duplicates are reported untriaged\n";
  const dropped = "warning: duplicate triage skipped: TYPESAFE_API_KEY is not set; the duplicates are reported untriaged\n" +
    "warning: duplicate triage skipped: OPENAI_API_KEY is not set; the duplicates are reported untriaged\n";
  assert.equal(unpaidStderr(dropped + kept + "tail without newline"), kept + "tail without newline");
});

test("option rules stop at --", () => {
  assert.ok(scanOnly(["--top", "1", "--duplicates"]));
  assert.ok(!scanOnly(["--", "--duplicates"]));
  assert.ok(!scanOnly(["--duplicates=true"]));
  assert.deepEqual(withoutOptions(["--show-unchanged", "a", "--", "--show-unchanged"], ["--show-unchanged"]),
    ["a", "--", "--show-unchanged"]);
});
