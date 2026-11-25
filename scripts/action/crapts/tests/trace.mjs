import childProcess from "node:child_process";
import { appendFileSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { PROVIDER_KEYS, PROVIDER_URLS } from "../cli.mjs";

// Names only: a trace never holds a key's value.
function providers(env) {
  const set = names => Object.entries(env).filter(([name, value]) => value && names.includes(name.toUpperCase()));
  return { keys: set(PROVIDER_KEYS).map(([name]) => name).sort(), urls: Object.fromEntries(set(PROVIDER_URLS)) };
}

const spawn = childProcess.spawnSync;
childProcess.spawnSync = function (command, args, options) {
  if (command === process.env.CRAPTS_BINARY)
    appendFileSync(process.env.ARG_TRACE, JSON.stringify({ args, cwd: options?.cwd || process.cwd(), ...providers(options?.env ?? process.env) }) + "\n");
  return spawn.call(this, command, args, options);
};
const execFile = childProcess.execFileSync;
childProcess.execFileSync = function (command, args, options) {
  if (command === "git" && args[0] === "worktree" && args[1] === "add" && providers(options?.env ?? process.env).keys.length)
    throw Error("worktree add received a provider key");
  const result = execFile.call(this, command, args, options);
  if (process.env.INJECT_CLEANUP_ERROR && command === "git" && args[0] === "worktree" && args[1] === "remove")
    throw Error("cleanup failed");
  return result;
};
syncBuiltinESMExports();
