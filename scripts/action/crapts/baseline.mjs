import { execFileSync } from "node:child_process";
import { copyFileSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { generateCoverage } from "./prepare.mjs";
import { withOptions, writeReport } from "./cli.mjs";

function git(...args) {
  return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trimEnd();
}

function configPaths(root, cwd) {
  const configs = [];
  for (let dir = cwd; ; dir = dirname(dir)) {
    configs.push(join(dir, ".crapts.toml"));
    if (dir === root) return configs;
    if (dir === dirname(dir)) throw Error(cwd + " is outside worktree " + root);
  }
}

function realDirectory(path) {
  try {
    return realpathSync.native(path);
  } catch (error) {
    if (error.code !== "ENOENT" || path === dirname(path)) throw error;
    return join(realDirectory(dirname(path)), basename(path));
  }
}

function baseOutput(name, root, worktree) {
  if (!isAbsolute(name)) return name;
  // Clearing stale coverage unlinks the directory entry, even when the file itself is a symlink.
  const path = join(realDirectory(dirname(name)), basename(name));
  const part = relative(root, path);
  if (isAbsolute(part) || part === ".." || part.startsWith(".." + sep)) return name;
  return join(worktree, part);
}

function generate(base, temp, bash) {
  const root = realpathSync.native(git("rev-parse", "--show-toplevel"));
  const head = realpathSync.native(process.cwd());
  const project = relative(root, head);
  const worktree = join(temp, "base-worktree");
  const config = configPaths(root, head).find(path => statSync(path, { throwIfNoEntry: false })?.isFile());
  git("worktree", "add", "--detach", worktree, base);
  let originalError;
  try {
    const cwd = join(worktree, project);
    if (!statSync(cwd, { throwIfNoEntry: false })?.isDirectory()) {
      console.log("::notice::Missing base directory " + project + " at " + base);
      return "";
    }
    const realTree = realpathSync.native(worktree);
    const realCwd = realpathSync.native(cwd);
    if (realCwd !== realTree && !realCwd.startsWith(realTree + sep)) throw Error("base directory leaves detached worktree");
    for (const path of configPaths(realTree, realCwd)) rmSync(path, { force: true });
    if (config) copyFileSync(config, join(worktree, relative(root, config)));
    const name = process.env.INPUT_COVERAGE_FILE;
    const output = baseOutput(name, root, worktree);
    const coverage = generateCoverage(cwd, "base " + base, bash, output);
    const args = JSON.parse(readFileSync(join(temp, "args.json")));
    const flags = new Set(["--fail-above", "--fail-regression", "--fail-changed"]);
    const end = args.includes("--") ? args.indexOf("--") : args.length;
    const scoring = args.filter((arg, i) => i >= end || !flags.has(arg));
    const report = join(temp, "baseline.json");
    writeReport(cwd, withOptions(scoring, ["--coverage", coverage, "--format", "json", "--all-rows"]), report, "base " + base);
    return report;
  } catch (error) {
    originalError = error;
    throw error;
  } finally {
    try {
      git("worktree", "remove", "--force", worktree);
    } catch (error) {
      if (!originalError) throw error;
      console.error("Could not remove base worktree: " + error.message);
    }
  }
}

try {
  const { CRAPTS_TEMP: temp, INPUT_BASELINE: supplied, INPUT_COVERAGE_COMMAND: command } = process.env;
  const path = join(temp, "prepared.json");
  const state = JSON.parse(readFileSync(path));
  if (supplied) {
    if (!statSync(supplied, { throwIfNoEntry: false })?.isFile()) throw Error("baseline: missing file " + supplied);
    state.baseline = resolve(supplied);
  } else {
    state.baseline = state.base && command ? generate(state.base, temp, process.argv[2]) : "";
  }
  writeFileSync(path, JSON.stringify(state));
} catch (error) {
  console.error(error.message);
  process.exitCode = 2;
}
