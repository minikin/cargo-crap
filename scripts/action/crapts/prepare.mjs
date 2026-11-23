import { spawnSync } from "node:child_process";
import { copyFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

function git(args, quiet = false) {
  const result = spawnSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", quiet ? "ignore" : "inherit"] });
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

function projectCommand(command, label, cwd, bash) {
  const result = spawnSync(bash, ["-c", command], { cwd, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(label + " failed with status " + (result.status ?? result.signal));
}

function file(path, label) {
  if (!statSync(path, { throwIfNoEntry: false })?.isFile()) throw Error(label + ": missing file " + path);
  return path;
}

export function generateCoverage(cwd, label, bash) {
  const { INPUT_INSTALL_COMMAND: install, INPUT_COVERAGE_COMMAND: command, INPUT_COVERAGE_FILE: name } = process.env;
  const path = resolve(cwd, name);
  if (install) projectCommand(install, label + " install command", cwd, bash);
  rmSync(path, { force: true });
  projectCommand(command, label + " coverage command", cwd, bash);
  return file(path, label + " coverage command did not produce coverage-file " + name);
}

function prepare() {
  const { INPUT_BASE, INPUT_COVERAGE, INPUT_COVERAGE_COMMAND, CRAPTS_TEMP } = process.env;
  const base = resolveBase(INPUT_BASE);
  if (!INPUT_COVERAGE && !INPUT_COVERAGE_COMMAND) throw Error("set coverage and coverage-command inputs to supply or generate coverage");
  const source = INPUT_COVERAGE ? file(INPUT_COVERAGE, "coverage") : generateCoverage(process.cwd(), "head", process.argv[2]);
  const coverage = join(CRAPTS_TEMP, "head.coverage");
  copyFileSync(source, coverage);
  writeFileSync(join(CRAPTS_TEMP, "prepared.json"), JSON.stringify({ base, coverage }));
}

if (resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { prepare(); } catch (error) { console.error(error.message); process.exitCode = 2; }
}
