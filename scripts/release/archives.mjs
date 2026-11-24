import { chmodSync, closeSync, openSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { basename, join } from "node:path";

export function archiveName(target) {
  return (
    "crapts-" +
    target.triple +
    (target.platform === "win32" ? ".zip" : ".tar.gz")
  );
}

export function extractBinary(archive, binary, target) {
  const fd = openSync(binary, "wx");
  const windows = target.platform === "win32";
  try {
    const member = basename(target.binary);
    const stdio = ["ignore", fd, "pipe"];
    if (windows && process.platform !== "win32")
      execFileSync("unzip", ["-p", archive, member], { stdio });
    else {
      // Git Bash can put GNU tar ahead of the Windows tar that supports zip.
      const tar =
        process.platform === "win32"
          ? join(process.env.SystemRoot, "System32", "tar.exe")
          : "tar";
      execFileSync(tar, ["-xOf", archive, member], { stdio });
    }
  } finally {
    closeSync(fd);
  }
  if (!statSync(binary).size) throw Error("empty binary in " + archive);
  if (!windows) chmodSync(binary, 0o755);
}
