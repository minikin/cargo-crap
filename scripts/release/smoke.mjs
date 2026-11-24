import assert from "node:assert/strict";
import { execFile, execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { once } from "node:events";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { parseArgs, promisify } from "node:util";
import resolver from "../../npm/crapts/lib/resolve.js";

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

async function waitForPackage(name, version) {
  const spec = name + "@" + version;
  const [npm, ...args] = npmCommand("npm");
  for (let attempt = 1; attempt <= 3; attempt++) {
    const result = spawnSync(
      npm,
      [
        ...args,
        "view",
        spec,
        "version",
        "--prefer-online",
        "--fetch-retries=0",
        "--fetch-timeout=10000",
      ],
      { cwd: dir, encoding: "utf8" },
    );
    if (result.error) throw result.error;
    if (result.status === 0 && result.stdout.trim() === version) return;
    if (
      attempt === 3 ||
      (result.status !== 0 && !/\b(E404|ETARGET)\b/.test(result.stderr))
    )
      throw Error(
        spec + ": " + (result.stderr.trim() || "version unavailable"),
      );
    await delay(3000 * attempt);
  }
}

async function smokeTriage(command) {
  const project = join(dir, "copies");
  const home = join(dir, "cache-home");
  mkdirSync(project);
  mkdirSync(home);
  for (const name of ["first", "second"])
    writeFileSync(
      join(project, name + ".ts"),
      `export function ${name}(n: number) { return n + 1; }\n`,
    );
  const config = join(project, ".crapts.toml");
  const detection = "[duplicates]\nmin-nodes = 0\n";
  writeFileSync(config, detection);
  const requests = [];
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    requests.push({
      path: req.url,
      method: req.method,
      body: Buffer.concat(chunks).toString(),
    });
    res.writeHead(200, { "content-type": "application/json" });
    res.end(
      JSON.stringify({
        answers: {
          duplication_kind: {
            type: "choice",
            choice: "same_logic",
            confidence: 0.95,
          },
          worth_extracting: { type: "score", score: 3, confidence: 1 },
          divergence_risk: { type: "noul", noul: 0.25 },
        },
      }),
    );
  });
  server.listen(0, "127.0.0.1");
  try {
    await once(server, "listening");
    const endpoint = "http://127.0.0.1:" + server.address().port;
    const env = {
      ...process.env,
      HOME: home,
      USERPROFILE: home,
      XDG_CACHE_HOME: home,
      LOCALAPPDATA: home,
      TYPESAFE_API_KEY: "release-smoke",
      TYPESAFE_BASE_URL: endpoint,
      OPENAI_API_KEY: "",
      OPENAI_BASE_URL: endpoint + "/v1",
      npm_config_offline: "true",
    };
    const scan = async () => {
      const { stdout } = await promisify(execFile)(
        command[0],
        [...command.slice(1), "--duplicates", "--format", "json"],
        { cwd: project, env, timeout: 60_000 },
      );
      const groups = JSON.parse(stdout).duplicate_groups;
      assert.equal(groups.length, 1);
      return groups[0];
    };
    assert.equal((await scan()).triage, undefined);
    assert.equal(requests.length, 0);
    writeFileSync(config, detection + "[duplicates.triage]\nenabled = true\n");
    assert.deepEqual((await scan()).triage, {
      kind: "same-logic",
      confidence: 0.95,
      worth_extracting_score: 3,
      worth_extracting: "should-be-one",
      divergence_risk: 0.25,
    });
    assert.equal(requests.length, 1);
    assert.equal(requests[0].method, "POST");
    assert.equal(requests[0].path, "/v1/systemone");
    assert.equal(JSON.parse(requests[0].body).model, "jev-latest");
    console.log(
      "Triage smoke passed (0 default requests, 1 verdict from 1 request).",
    );
  } finally {
    server.close();
  }
}

try {
  if (!values.version || Boolean(values.npm) === Boolean(values.binary))
    throw Error("expected --version and one of --npm or --binary");
  let command;
  if (values.npm) {
    writeFileSync(join(dir, "package.json"), '{"private":true}');
    const spec = "@minikin/crapts@" + values.version;
    const libc = process.platform === "linux" ? resolver.detectLibc() : "none";
    const host = resolver.resolveTarget(process.platform, process.arch, libc);
    await waitForPackage("@minikin/crapts", values.version);
    await waitForPackage(host.name, values.version);
    const [npm, ...args] = npmCommand("npm");
    execFileSync(
      npm,
      [
        ...args,
        "install",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--prefer-online",
        spec,
      ],
      { cwd: dir, stdio: "inherit" },
    );
    if (values["expect-package"]) {
      const local = createRequire(join(dir, "package.json"));
      const { resolveTarget } = local("@minikin/crapts/lib/resolve.js");
      const installed = resolveTarget(process.platform, process.arch, libc);
      assert.equal(installed.name, values["expect-package"]);
      local.resolve(installed.name + "/" + installed.binary);
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
  await smokeTriage(command);
} finally {
  rmSync(dir, { force: true, recursive: true });
}
