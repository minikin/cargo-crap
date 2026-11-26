"use strict";
const { spawnSync } = require("node:child_process");

const targets = [
  ["darwin", "arm64", "none", "darwin-arm64", "aarch64-apple-darwin"],
  ["darwin", "x64", "none", "darwin-x64", "x86_64-apple-darwin"],
  ["linux", "x64", "glibc", "linux-x64-gnu", "x86_64-unknown-linux-gnu"],
  ["linux", "arm64", "glibc", "linux-arm64-gnu", "aarch64-unknown-linux-gnu"],
  ["linux", "x64", "musl", "linux-x64-musl", "x86_64-unknown-linux-musl"],
  ["win32", "x64", "none", "win32-x64-msvc", "x86_64-pc-windows-msvc"],
].map(([platform, arch, libc, suffix, triple]) => ({
  platform,
  arch,
  libc,
  name: "@minikin_/crapts-" + suffix,
  triple,
  binary: platform === "win32" ? "bin/crapts.exe" : "bin/crapts",
}));

function lddVersion() {
  const { stdout, stderr } = spawnSync("ldd", ["--version"], {
    encoding: "utf8",
  });
  return [stdout, stderr].join("\n");
}

function detectLibc(
  report = () => process.report.getReport(),
  ldd = lddVersion,
) {
  try {
    const data = report();
    const glibc = data?.header?.glibcVersionRuntime;
    if (typeof glibc === "string" && glibc.trim()) return "glibc";
    if (data?.sharedObjects?.some((path) => /musl/i.test(path))) return "musl";
  } catch {
    /* Node builds may omit runtime reports. */
  }
  try {
    const version = ldd();
    if (/musl/i.test(version)) return "musl";
    if (/GNU libc|GLIBC/i.test(version)) return "glibc";
  } catch {
    /* Minimal container images may omit ldd. */
  }
  return "unknown";
}

function resolveTarget(platform, arch, libc) {
  if (platform === "linux" && libc === "unknown")
    throw Error(platform + " " + arch + ": libc could not be detected");
  const target = targets.find(
    (t) => t.platform === platform && t.arch === arch && t.libc === libc,
  );
  if (!target)
    throw Error("unsupported target " + [platform, arch, libc].join(" "));
  return target;
}

module.exports = { detectLibc, resolveTarget, targets };
