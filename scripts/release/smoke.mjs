import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    npm: { type: "boolean" },
    binary: { type: "string" },
    "expect-package": { type: "string" },
  },
});
const dir = mkdtempSync(join(tmpdir(), "crapts-smoke-"));
function npmCommand(tool) {
  if (process.platform !== "win32") return [tool];
  return [
    process.execPath,
    join(dirname(process.execPath), "node_modules/npm/bin", tool + "-cli.js"),
  ];
}

try {
  if (!values.version || Boolean(values.npm) === Boolean(values.binary))
    throw Error("expected --version and one of --npm or --binary");
  let command;
  if (values.npm) {
    writeFileSync(join(dir, "package.json"), '{"private":true}');
    const spec = "@minikin/crapts@" + values.version;
    const [npm, ...args] = npmCommand("npm");
    execFileSync(
      npm,
      [...args, "install", "--ignore-scripts", "--no-audit", "--no-fund", spec],
      { cwd: dir, stdio: "inherit" },
    );
    if (values["expect-package"]) {
      const local = createRequire(join(dir, "package.json"));
      const { resolveTarget, detectLibc } = local(
        "@minikin/crapts/lib/resolve.js",
      );
      const host = resolveTarget(process.platform, process.arch, detectLibc());
      assert.equal(host.name, values["expect-package"]);
      local.resolve(host.name + "/" + host.binary);
    }
    command = [
      ...npmCommand("npx"),
      "--yes",
      "--ignore-scripts",
      "--package",
      spec,
      "--",
      "crapts",
    ];
  } else command = [resolve(values.binary)];
  writeFileSync(
    join(dir, "example.ts"),
    "export function risk(b: boolean) { return b ? 1 : 0; }\n",
  );
  writeFileSync(
    join(dir, "coverage.info"),
    "SF:example.ts\nDA:1,0\nend_of_record\n",
  );
  const args = ["--coverage", "coverage.info", "example.ts"];
  for (const [argv, code] of [
    [["--version"], 0],
    [args, 0],
    [[...args, "--fail-above", "--threshold", "0"], 1],
    [["--invalid-option"], 2],
  ]) {
    const result = spawnSync(command[0], [...command.slice(1), ...argv], {
      cwd: dir,
      encoding: "utf8",
    });
    if (result.error) throw result.error;
    assert.equal(
      result.status,
      code,
      argv.join(" ") + "\n" + result.stdout + result.stderr,
    );
    if (argv[0] === "--version")
      assert.equal(result.stdout.trim(), "crapts " + values.version);
  }
} finally {
  rmSync(dir, { force: true, recursive: true });
}
