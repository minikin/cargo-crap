import { spawnSync } from "node:child_process";
import { closeSync, openSync } from "node:fs";

export function withOptions(args, options) {
  const at = args.indexOf("--");
  return at < 0 ? [...args, ...options] : [...args.slice(0, at), ...options, ...args.slice(at)];
}

export function writeReport(cwd, args, destination, label) {
  const output = openSync(destination, "w");
  try {
    const result = spawnSync(process.env.CRAPTS_BINARY, args, { cwd, stdio: ["ignore", output, "inherit"] });
    if (result.error) throw result.error;
    if (result.status !== 0 && result.status !== 1)
      throw Error(label + " crapts run failed with status " + (result.status ?? result.signal));
    return result.status;
  } finally {
    closeSync(output);
  }
}
