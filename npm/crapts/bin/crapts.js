#!/usr/bin/env node
"use strict";
const { spawnSync } = require("node:child_process");
const { detectLibc, resolveTarget } = require("../lib/resolve.js");

function run(
  argv,
  {
    platform = process.platform,
    arch = process.arch,
    libc = platform === "linux" ? detectLibc() : "none",
    locate = require.resolve,
    spawn = spawnSync,
    error = console.error,
    signal = (value) => process.kill(process.pid, value),
  } = {},
) {
  try {
    const target = resolveTarget(platform, arch, libc);
    let path;
    try {
      path = locate(target.name + "/" + target.binary);
    } catch (e) {
      if (e.code !== "MODULE_NOT_FOUND") throw e;
      throw Error(
        "Missing " +
          target.name +
          ". Reinstall with optional dependencies enabled.",
      );
    }
    const child = spawn(path, argv, { stdio: "inherit" });
    if (child.error) throw Error(path + ": " + child.error.message);
    if (child.signal) signal(child.signal);
    return child.status ?? 2;
  } catch (e) {
    error("crapts: " + e.message);
    return 2;
  }
}

if (require.main === module) process.exitCode = run(process.argv.slice(2));
module.exports = { run };
