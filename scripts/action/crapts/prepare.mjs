import { spawnSync } from "node:child_process";
import { copyFileSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { scanOnly, withoutProviderKeys } from "./cli.mjs";

function git(args, quiet = false) {
  const result = spawnSync("git", args, { env: withoutProviderKeys(process.env), encoding: "utf8", stdio: ["ignore", "pipe", quiet ? "ignore" : "inherit"] });
  if (result.error) throw result.error;
  return result;
}

function resolveBase(revision) {
  if (!revision) return "";
  if (revision.startsWith("-")) throw Error("invalid base revision " + revision);
  const commit = ref => git(["rev-parse", "--verify", "--end-of-options", ref + "^{commit}"], true).stdout.trim();
  const head = commit("HEAD");
  if (!head) throw Error("cannot compare " + revision + " without HEAD");
  let base = commit(revision);
  if (!base) {
    const remoteRef = revision.replace(/^(?:refs\/remotes\/)?origin\//, "refs/heads/");
    if (git(["fetch", "--no-tags", "--depth=1", "origin", remoteRef]).status !== 0 || !(base = commit("FETCH_HEAD")))
      throw Error("could not fetch base " + revision + " from origin");
  }
  const connected = () => git(["merge-base", base, head], true).status === 0;
  if (connected()) return base;
  let deepened = false;
  for (const depth of ["--deepen=50", "--deepen=200", "--deepen=1000", "--unshallow"]) {
    if (git(["rev-parse", "--is-shallow-repository"]).stdout.trim() !== "true") break;
    if (git(["fetch", "--no-tags", depth, "origin", base, head]).status !== 0)
      throw Error("failed fetch from origin for base " + revision + " and HEAD (" + depth + ")");
    deepened = true;
    if (connected()) return base;
  }
  throw Error("no merge base between " + revision + " and HEAD" + (deepened ? " after unshallowing" : "") +
    ": the base is not in this repository's history");
}

function projectCommand(command, label, cwd, bash, coverageFile) {
  const env = { ...withoutProviderKeys(process.env), INPUT_COVERAGE_FILE: coverageFile };
  const result = spawnSync(bash, ["-c", command], { cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(label + " failed with status " + (result.status ?? result.signal));
}

function file(path, label) {
  if (!statSync(path, { throwIfNoEntry: false })?.isFile()) throw Error(label + ": missing file " + path);
  return path;
}

// `guard` runs after the install command, before the stale file is unlinked,
// and again before the result is read: the commands can move what the path names.
export function generateCoverage(cwd, label, bash, name = process.env.INPUT_COVERAGE_FILE, guard = () => {}, hint = "") {
  const { INPUT_INSTALL_COMMAND: install, INPUT_COVERAGE_COMMAND: command } = process.env;
  const path = resolve(cwd, name);
  if (install) projectCommand(install, label + " install command", cwd, bash, name);
  guard(path);
  rmSync(path, { force: true });
  projectCommand(command, label + " coverage command", cwd, bash, name);
  guard(path, true);
  return file(path, label + " coverage command did not produce coverage-file " + name + hint);
}

function prepare() {
  const { INPUT_BASE, INPUT_COVERAGE, INPUT_COVERAGE_COMMAND, CRAPTS_TEMP } = process.env;
  const base = resolveBase(INPUT_BASE);
  let coverage = "";
  if (INPUT_COVERAGE || INPUT_COVERAGE_COMMAND) {
    const source = INPUT_COVERAGE ? file(INPUT_COVERAGE, "coverage") : generateCoverage(process.cwd(), "head", process.argv[2]);
    coverage = join(CRAPTS_TEMP, "head.coverage");
    copyFileSync(source, coverage);
  } else if (!scanOnly(JSON.parse(readFileSync(join(CRAPTS_TEMP, "args.json"))))) {
    throw Error("set coverage or coverage-command inputs to supply or generate coverage");
  }
  writeFileSync(join(CRAPTS_TEMP, "prepared.json"), JSON.stringify({ base, coverage }));
}

if (resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { prepare(); } catch (error) { console.error(error.message); process.exitCode = 2; }
}
