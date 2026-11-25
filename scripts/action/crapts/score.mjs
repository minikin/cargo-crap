import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { jobSummary } from "./delivery.mjs";
import { optionsEnd, withOptions, withoutOptions, writeReport } from "./cli.mjs";

function count(summary, key, optional = false) {
  const value = summary[key];
  if (value === null || (optional && value === undefined)) return "";
  if (!Number.isSafeInteger(value) || value < 0) throw Error("invalid summary." + key);
  return String(value);
}

// A comparison flag without its comparison makes crapts exit 2. Config gates
// stay dormant in that case, so the same flags given through args do too.
function dormant(args, base, baseline) {
  const needs = [["--show-unchanged", baseline, "a baseline"], ["--fail-regression", baseline, "a baseline"],
    ["--fail-changed", base, "a base"]];
  const given = args.slice(0, optionsEnd(args));
  const dropped = needs.filter(([flag, present]) => !present && given.includes(flag));
  for (const [flag, , what] of dropped) console.log("::notice::args: " + flag + " needs " + what + ", and this run has none");
  return withoutOptions(args, dropped.map(([flag]) => flag));
}

function score() {
  const env = process.env;
  const path = name => join(env.CRAPTS_TEMP, name);
  const { coverage, base, baseline } = JSON.parse(readFileSync(path("prepared.json")));
  const args = dormant(JSON.parse(readFileSync(path("args.json"))), base, baseline);
  const trusted = { TYPESAFE_BASE_URL: env.CRAPTS_TRUSTED_TYPESAFE_BASE_URL || undefined,
    OPENAI_BASE_URL: env.CRAPTS_TRUSTED_OPENAI_BASE_URL || undefined };
  const options = coverage ? ["--coverage", coverage] : [];
  if (base) options.push("--base", base);
  if (baseline) options.push("--baseline", baseline);
  if (baseline && env.INPUT_FAIL_REGRESSION === "true") options.push("--fail-regression");
  if (base && env.INPUT_FAIL_CHANGED === "true") options.push("--fail-changed");
  if (env.INPUT_FAIL_ABOVE === "true") options.push("--fail-above");
  if (env.CRAPTS_REVISION) options.push("--commit-ref", env.CRAPTS_REVISION);
  if (env.CRAPTS_PR_NUMBER) options.push("--comment-marker", env.INPUT_COMMENT_MARKER);
  if (base && !baseline) console.log("::notice::no base coverage and no baseline: only the diff gate ran");

  const report = path("report.md");
  const run = (flags, file, paid) =>
    writeReport(process.cwd(), withOptions(args, [...options, ...flags]), file, "head", { paid });
  run(["--print-config"], path("settings.json"));
  run(["--format", env.CRAPTS_PR_NUMBER ? "pr-comment" : "markdown"], report, trusted);
  const status = run(["--format", "json", "--all-rows"], path("gate.json"));
  const { summary } = JSON.parse(readFileSync(path("gate.json")));
  const outputs = { crappy: count(summary, "crappy"), regressed: count(summary, "regressed"),
    changed: count(summary, "changed_failures"), "signal-regressions": count(summary, "signal_regressions", true),
    report, status };
  const settings = readFileSync(path("settings.json"), "utf8");
  appendFileSync(env.GITHUB_STEP_SUMMARY, jobSummary(readFileSync(report, "utf8"), settings));
  appendFileSync(env.GITHUB_OUTPUT, Object.entries(outputs).map(([key, value]) => key + "=" + value + "\n").join(""));
}

try { score(); } catch (error) { console.error(error.message); process.exitCode = 2; }
