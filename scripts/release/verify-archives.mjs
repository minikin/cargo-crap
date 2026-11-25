import { createHash } from "node:crypto";
import { basename, join } from "node:path";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { parseArgs } from "node:util";
import resolver from "../../npm/crapts/lib/resolve.js";
import { archiveName, extractBinary } from "./archives.mjs";

function compareStaged(archive, target, staged) {
  const scratch = mkdtempSync(join(tmpdir(), "crapts-verify-"));
  try {
    const member = join(scratch, basename(target.binary));
    extractBinary(archive, member, target);
    const packaged = join(staged, basename(target.name), target.binary);
    if (!readFileSync(member).equals(readFileSync(packaged)))
      throw Error("staged binary differs for " + target.triple);
  } finally {
    rmSync(scratch, { force: true, recursive: true });
  }
}

function verify(dir, triple, staged) {
  const expected = new Map(
    resolver.targets.map((target) => [archiveName(target), target]),
  );
  const sums = new Map();
  for (const line of readFileSync(join(dir, "SHA256SUMS"), "utf8")
    .trimEnd()
    .split(/\r?\n/)) {
    const match = /^([a-f0-9]{64}) [ *](.+)$/.exec(line);
    if (!match || !expected.has(match[2]) || sums.has(match[2]))
      throw Error("invalid SHA256SUMS entry: " + line);
    sums.set(match[2], match[1]);
  }
  if (sums.size !== expected.size)
    throw Error("SHA256SUMS must cover all six crapts archives");
  if (triple && !resolver.targets.some((t) => t.triple === triple))
    throw Error("unknown target " + triple);
  for (const [name, target] of expected) {
    if (triple && target.triple !== triple) continue;
    const path = join(dir, name);
    const hash = createHash("sha256").update(readFileSync(path)).digest("hex");
    if (sums.get(name) !== hash) throw Error("checksum mismatch: " + name);
    if (staged) compareStaged(path, target, staged);
    if (triple) extractBinary(path, join(dir, basename(target.binary)), target);
  }
}

try {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: { staged: { type: "string" } },
  });
  if (positionals.length < 1 || positionals.length > 2)
    throw Error("expected archive directory and optional target");
  verify(positionals[0], positionals[1], values.staged);
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
}
