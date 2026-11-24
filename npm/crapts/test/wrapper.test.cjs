const assert = require("node:assert/strict");
const { test } = require("node:test");
const { detectLibc, resolveTarget } = require("../lib/resolve.js");
const { run } = require("../bin/crapts.js");

const triples = [
  ["darwin", "arm64", "none", "darwin-arm64"],
  ["darwin", "x64", "none", "darwin-x64"],
  ["linux", "x64", "glibc", "linux-x64-gnu"],
  ["linux", "arm64", "glibc", "linux-arm64-gnu"],
  ["linux", "x64", "musl", "linux-x64-musl"],
  ["win32", "x64", "none", "win32-x64-msvc"],
];

test("wrapper selects the platform package for every supported target", () => {
  for (const [platform, arch, libc, suffix] of triples) {
    const target = resolveTarget(platform, arch, libc);
    assert.equal(target.name, "@minikin/crapts-" + suffix);
    assert.equal(
      target.binary,
      platform === "win32" ? "bin/crapts.exe" : "bin/crapts",
    );
  }
});

test("Linux host libc detection distinguishes glibc, musl and inconclusive evidence", () => {
  for (const [report, ldd, expected] of [
    [{ header: { glibcVersionRuntime: "2.36" } }, "musl", "glibc"],
    [{ sharedObjects: ["/lib/ld-musl-x86_64.so.1"] }, "GLIBC", "musl"],
    [{}, "musl libc", "musl"],
    [null, "GNU libc 2.36", "glibc"],
    [{ header: { glibcVersionRuntime: "" } }, "ldd (GLIBC) 2.28", "glibc"],
    [{}, "unrecognized", "unknown"],
  ])
    assert.equal(
      detectLibc(
        () => report,
        () => ldd,
      ),
      expected,
    );
  const broken = () => {
    throw Error("unavailable");
  };
  assert.equal(detectLibc(broken, broken), "unknown");
});

test("platform resolution is total and injective across generated triples", () => {
  const names = new Set();
  for (const os of ["darwin", "linux", "win32", "freebsd", "", "__proto__"]) {
    for (const arch of ["x64", "arm64", "ia32", "riscv64", ""]) {
      for (const libc of ["none", "musl", "glibc", "unknown", ""]) {
        if (triples.some(([p, a, l]) => p === os && a === arch && l === libc)) {
          names.add(resolveTarget(os, arch, libc).name);
        } else assert.throws(() => resolveTarget(os, arch, libc));
      }
    }
  }
  assert.equal(names.size, 6);
  const { randomBytes } = require("node:crypto");
  for (let i = 0; i < 128; i++) {
    const values = triples[i % triples.length].slice(0, 3);
    values[i % 3] += "-" + randomBytes(4).toString("hex");
    assert.throws(() => resolveTarget(...values), /unsupported/);
  }
});

test("wrapper diagnoses unsupported platforms, unknown libc, missing packages and spawn errors with exit 2", () => {
  const cases = [
    ["linux", "arm64", "musl", /unsupported.*linux.*arm64.*musl/i],
    ["linux", "x64", "unknown", /linux.*x64.*libc.*detect/i],
    ["freebsd", "x64", "none", /unsupported.*freebsd.*x64/i],
    ["win32", "arm64", "none", /unsupported.*win32.*arm64/i],
    [
      "darwin",
      "arm64",
      "none",
      /@minikin\/crapts-darwin-arm64.*optional dependencies/i,
    ],
    ["darwin", "x64", "none", /path to binary.*EACCES/i, "EACCES"],
    ["darwin", "x64", "none", /path to binary.*ENOENT/i, "ENOENT"],
  ];
  for (const [platform, arch, libc, message, osError] of cases) {
    let stderr = "";
    let spawned = false;
    const status = run(["--version"], {
      platform,
      arch,
      libc,
      locate() {
        assert.equal(platform, "darwin");
        if (arch === "x64") return "/path to binary";
        throw Object.assign(Error("missing"), { code: "MODULE_NOT_FOUND" });
      },
      spawn() {
        spawned = true;
        if (!osError) throw Error("unexpected spawn");
        return { error: Error(osError) };
      },
      error: (line) => {
        stderr += line;
      },
    });
    assert.equal(status, 2);
    assert.match(stderr, message);
    assert.equal(spawned, Boolean(osError));
  }
});

test("child signal is re-raised", () => {
  let signal;
  const status = run([], {
    platform: "darwin",
    arch: "arm64",
    libc: "none",
    locate: () => "/bin/crapts",
    spawn: () => ({ signal: "SIGTERM" }),
    signal: (value) => {
      signal = value;
    },
  });
  assert.equal(signal, "SIGTERM");
  assert.equal(status, 2);
});
