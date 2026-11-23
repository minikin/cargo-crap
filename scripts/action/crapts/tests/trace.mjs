import childProcess from "node:child_process";
import { appendFileSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";

const spawn = childProcess.spawnSync;
childProcess.spawnSync = function (command, args, options) {
  if (command === process.env.CRAPTS_BINARY)
    appendFileSync(process.env.ARG_TRACE, JSON.stringify({ args, cwd: options?.cwd || process.cwd() }) + "\n");
  return spawn.call(this, command, args, options);
};
syncBuiltinESMExports();
