import { createHash } from "node:crypto";
import { basename, join } from "node:path";
import { readFileSync } from "node:fs";
import resolver from "../../npm/crapts/lib/resolve.js";
import { archiveName, extractBinary } from "./archives.mjs";

function verify(dir, triple) {
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
    if (triple) extractBinary(path, join(dir, basename(target.binary)), target);
  }
}

try {
  verify(...process.argv.slice(2));
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
}
