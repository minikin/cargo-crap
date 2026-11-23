import assert from "node:assert/strict";
import { test } from "node:test";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
  statSync,
  renameSync,
  copyFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { once } from "node:events";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const release = join(root, "scripts/release");
const rows = [
  ["aarch64-apple-darwin", "darwin-arm64", "darwin", "arm64"],
  ["x86_64-apple-darwin", "darwin-x64", "darwin", "x64"],
  ["x86_64-unknown-linux-gnu", "linux-x64-gnu", "linux", "x64", "glibc"],
  ["aarch64-unknown-linux-gnu", "linux-arm64-gnu", "linux", "arm64", "glibc"],
  ["x86_64-unknown-linux-musl", "linux-x64-musl", "linux", "x64", "musl"],
  ["x86_64-pc-windows-msvc", "win32-x64-msvc", "win32", "x64"],
];
const version = "0.1.0-rc.1";
const name = (suffix) => "@minikin/crapts-" + suffix;
const manifest = (dir) => JSON.parse(readFileSync(join(dir, "package.json")));
const windows = process.platform === "win32";

function npm(args, cwd) {
  const cli = windows
    ? join(dirname(process.execPath), "node_modules/npm/bin/npm-cli.js")
    : "npm";
  return execFileSync(
    windows ? process.execPath : cli,
    windows ? [cli, ...args] : args,
    { cwd, encoding: "utf8" },
  );
}

function fixture(t, native) {
  const dir = mkdtempSync(join(tmpdir(), "crapts-npm-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const archives = join(dir, "archives");
  const out = join(dir, "staged");
  mkdirSync(archives);
  for (const [target, suffix, os] of rows) {
    const bin = os === "win32" ? "crapts.exe" : "crapts";
    if (native?.name === name(suffix))
      copyFileSync(native.path, join(dir, bin));
    else writeFileSync(join(dir, bin), target);
    const archive = join(
      archives,
      "crapts-" + target + (os === "win32" ? ".zip" : ".tar.gz"),
    );
    if (os === "win32")
      execFileSync(
        windows ? "python" : "python3",
        ["-m", "zipfile", "-c", archive, bin],
        { cwd: dir },
      );
    else execFileSync("tar", ["czf", archive, bin], { cwd: dir });
  }
  const stage = () =>
    execFileSync(
      process.execPath,
      [
        join(release, "stage-npm.mjs"),
        "--archives",
        archives,
        "--out",
        out,
        "--version",
        version,
      ],
      { stdio: "pipe" },
    );
  stage();
  return { dir, out, archives, stage };
}

test("staged npm packages declare platform filters, provenance metadata and zero install scripts", (t) => {
  const { out, archives, stage } = fixture(t);
  const wrapper = manifest(join(out, "crapts"));
  assert.deepEqual(
    wrapper.optionalDependencies,
    Object.fromEntries(rows.map(([, suffix]) => [name(suffix), version])),
  );
  assert.equal(wrapper.bin.crapts, "bin/crapts.js");
  for (const [target, suffix, os, arch, libc] of rows) {
    const folder = join(out, "crapts-" + suffix);
    const pkg = manifest(folder);
    for (const item of [wrapper, pkg]) {
      assert.equal(item.version, version);
      assert.equal(item.license, "MIT");
      assert.equal(
        item.repository.url,
        "git+https://github.com/minikin/cargo-crap.git",
      );
      assert.equal(item.publishConfig.access, "public");
      for (const hook of ["preinstall", "install", "postinstall", "prepare"])
        assert.equal(item.scripts?.[hook], undefined);
    }
    assert.equal(pkg.name, name(suffix));
    assert.deepEqual(pkg.os, [os]);
    assert.deepEqual(pkg.cpu, [arch]);
    assert.deepEqual(pkg.libc, libc && [libc]);
    assert.deepEqual(pkg.files, ["bin/"]);
    const bin = join(folder, "bin", os === "win32" ? "crapts.exe" : "crapts");
    assert.equal(readFileSync(bin, "utf8"), target);
    if (!windows && os !== "win32")
      assert.equal(statSync(bin).mode & 0o777, 0o755);
    const file = join(
      archives,
      "crapts-" + target + (os === "win32" ? ".zip" : ".tar.gz"),
    );
    renameSync(file, file + ".missing");
    assert.throws(stage, new RegExp(target));
    renameSync(file + ".missing", file);
  }
});

test("wrapper forwards argv, stdio and exit codes 0, 1 and 2 without install scripts", async (t) => {
  const { resolveTarget, detectLibc } = await import("../lib/resolve.js");
  const host = resolveTarget(
    process.platform,
    process.arch,
    process.platform === "linux" ? detectLibc() : "none",
  );
  const binary = resolve(
    process.env.CRAPTS_BINARY ||
      join(root, "target/debug", windows ? "crapts.exe" : "crapts"),
  );
  const { dir, out } = fixture(t, { name: host.name, path: binary });
  const packed = ["crapts", host.name.split("/")[1]].map(
    (pkg) =>
      JSON.parse(
        npm(
          ["pack", "--json", "--ignore-scripts", "--pack-destination", dir],
          join(out, pkg),
        ),
      )[0].filename,
  );
  writeFileSync(join(dir, "package.json"), '{"private":true}');
  npm(
    [
      "install",
      "--offline",
      "--no-audit",
      "--no-fund",
      "--ignore-scripts",
      ...packed,
    ],
    dir,
  );
  writeFileSync(
    join(dir, "with spaces.ts"),
    "export function f(b: boolean) { return b ? 1 : 0; }\n",
  );
  writeFileSync(
    join(dir, "coverage.info"),
    "SF:with spaces.ts\nDA:1,0\nend_of_record\n",
  );
  const cli = join(dir, "node_modules/@minikin/crapts/bin/crapts.js");
  const cases = [
    [["--version"], 0],
    [["--coverage", "coverage.info", "--", "with spaces.ts"], 0],
    [
      [
        "--coverage",
        "coverage.info",
        "--fail-above",
        "--threshold",
        "0",
        "with spaces.ts",
      ],
      1,
    ],
    [["--unknown-option"], 2],
    [["--coverage", "coverage.info", "--", "absent spaced.ts"], 2],
  ];
  for (const [args, status] of cases) {
    const direct = spawnSync(binary, args, { cwd: dir });
    const wrapped = spawnSync(process.execPath, [cli, ...args], { cwd: dir });
    assert.equal(direct.status, status, direct.stderr?.toString());
    assert.equal(wrapped.status, status, wrapped.stderr?.toString());
    assert.deepEqual(wrapped.stdout, direct.stdout);
    assert.deepEqual(wrapped.stderr, direct.stderr);
  }
});

test("npm publish retry verifies integrity and provenance before skipping existing packages", async (t) => {
  const { publishRelease } =
    await import("../../../scripts/release/publish-npm.mjs");
  const { out } = fixture(t);
  const repo = "minikin/cargo-crap";
  const commit = "a".repeat(40);
  const existing = new Map();
  const uploaded = [];
  const slsa = "https://slsa.dev/provenance/v1";
  const server = createServer((req, res) => {
    const url = decodeURIComponent(req.url);
    const info = existing.get(url.split("/").slice(1, 3).join("/"));
    if (!info) {
      res.writeHead(404).end();
      return;
    }
    if (url.endsWith("/attestation")) {
      const statement = {
        predicateType: slsa,
        subject: [{ name: info.purl, digest: { sha512: info.hex } }],
        predicate: {
          buildDefinition: {
            externalParameters: { workflow: { repository: info.repo } },
            resolvedDependencies: [
              {
                uri:
                  "git+https://github.com/" +
                  repo +
                  "@refs/tags/crapts-v" +
                  version,
                digest: { gitCommit: info.commit },
              },
            ],
          },
        },
      };
      res.end(
        JSON.stringify({
          attestations: [
            {
              predicateType: slsa,
              bundle: {
                dsseEnvelope: {
                  payload: Buffer.from(JSON.stringify(statement)).toString(
                    "base64",
                  ),
                },
              },
            },
          ],
        }),
      );
    } else
      res.end(
        JSON.stringify({
          dist: {
            integrity: info.integrity,
            attestations: { url: registry + "/" + info.name + "/attestation" },
          },
        }),
      );
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => server.close());
  const registry = "http://127.0.0.1:" + server.address().port;
  const publish = async (archive, args, pkg) => {
    for (const part of [
      "--provenance",
      "--access",
      "public",
      "--tag",
      "next",
      "--registry",
      registry,
    ])
      assert.ok(args.includes(part));
    const integrity =
      "sha512-" +
      (await import("node:crypto"))
        .createHash("sha512")
        .update(readFileSync(archive))
        .digest("base64");
    existing.set(pkg, {
      name: pkg,
      integrity,
      hex: Buffer.from(integrity.slice(7), "base64").toString("hex"),
      purl: "pkg:npm/" + pkg.replace("@", "%40") + "@" + version,
      repo: "https://github.com/" + repo,
      commit,
    });
    uploaded.push(pkg);
  };
  const run = () =>
    publishRelease(
      { stagedDir: out, version, commit, repo, registry, tag: "next" },
      { publish, verifyBundle: async () => {} },
    );
  await run();
  assert.equal(uploaded.length, 7);
  assert.equal(uploaded.pop(), "@minikin/crapts");
  existing.delete("@minikin/crapts");
  const first = existing.get(uploaded[1]);
  for (const [field, wrong, error] of [
    ["integrity", "changed", /integrity/i],
    ["commit", "bad", /provenance/i],
    ["repo", "another", /provenance/i],
  ]) {
    const saved = first[field];
    first[field] = wrong;
    uploaded.length = 0;
    await assert.rejects(
      run,
      (e) => e.message.includes(first.name) && error.test(e.message),
    );
    assert.deepEqual(uploaded, []);
    first[field] = saved;
  }
  existing.delete(first.name);
  await run();
  assert.deepEqual(uploaded, [first.name, "@minikin/crapts"]);
});
