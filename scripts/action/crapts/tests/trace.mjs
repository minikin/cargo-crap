import childProcess from "node:child_process";
import { appendFileSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";

const spawn = childProcess.spawnSync;
childProcess.spawnSync = function (command, args, options) {
  if (command === process.env.CRAPTS_BINARY)
    appendFileSync(process.env.ARG_TRACE, JSON.stringify({ args, cwd: options?.cwd || process.cwd() }) + "\n");
  return spawn.call(this, command, args, options);
};
const execFile = childProcess.execFileSync;
childProcess.execFileSync = function (command, args, options) {
  const result = execFile.call(this, command, args, options);
  if (process.env.INJECT_CLEANUP_ERROR && command === "git" && args[0] === "worktree" && args[1] === "remove")
    throw Error("cleanup failed");
  return result;
};
syncBuiltinESMExports();
