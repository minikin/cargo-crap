import { execFileSync } from "node:child_process";
import { copyFileSync, lstatSync, mkdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { generateCoverage } from "./prepare.mjs";
import { withOptions, withoutOptions, withoutProviderKeys, writeReport } from "./cli.mjs";

function git(...args) {
  return execFileSync("git", args, { env: withoutProviderKeys(process.env), encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trimEnd();
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

function inside(path, root) {
  const part = relative(root, path);
  return !isAbsolute(part) && part !== ".." && !part.startsWith(".." + sep);
}

// The base run never writes outside its own trees. A path in the head checkout
// moves into the worktree, and anything else, such as a shared runner-temp file,
// goes to a directory only this run uses.
function baseTarget(name, cwd, root, worktree, privateDir) {
  let path = isAbsolute(name) ? null : resolve(cwd, name);
  if (isAbsolute(name)) {
    // Clearing stale coverage unlinks the directory entry, even when the file itself is a symlink.
    const real = join(realDirectory(dirname(name)), basename(name));
    if (inside(real, root)) path = join(worktree, relative(root, real));
  }
  if (path && inside(path, worktree)) return { path, within: worktree };
  return { path: join(privateDir, basename(name)), within: privateDir };
}

// Both roots are resolved before any project command runs, so a command that
// swaps a directory for a symlink cannot move the root the check compares with.
function outputGuard(name, label, realWithin) {
  return (path, produced) => {
    const parent = realDirectory(dirname(path));
    if (parent !== realWithin && !parent.startsWith(realWithin + sep))
      throw Error(label + " coverage-file " + name + " leaves the base run's directory through " + parent);
    if (produced && lstatSync(path, { throwIfNoEntry: false })?.isSymbolicLink())
      throw Error(label + " coverage-file " + name + " is a symbolic link");
  };
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
    const privateDir = join(temp, "base-output");
    rmSync(privateDir, { recursive: true, force: true });
    mkdirSync(privateDir);
    const target = baseTarget(name, cwd, root, worktree, privateDir);
    const realWithin = target.within === worktree ? realTree : realpathSync.native(privateDir);
    const label = "base " + base;
    const hint = target.within === privateDir ? ": write the path in $INPUT_COVERAGE_FILE" : "";
    const output = target.within === worktree && !isAbsolute(name) ? name : target.path;
    const coverage = generateCoverage(cwd, label, bash, output, outputGuard(name, label, realWithin), hint);
    const args = JSON.parse(readFileSync(join(temp, "args.json")));
    const scoring = withoutOptions(args, ["--fail-above", "--fail-regression", "--fail-changed", "--show-unchanged"]);
    const report = join(temp, "baseline.json");
    writeReport(cwd, withOptions(scoring, ["--coverage", coverage, "--format", "json", "--all-rows"]), report, "base " + base);
    return report;
  } catch (error) {
    originalError = error;
    throw error;
  } finally {
    try {
      git("worktree", "remove", "--force", worktree);
      rmSync(worktree, { recursive: true, force: true });
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
