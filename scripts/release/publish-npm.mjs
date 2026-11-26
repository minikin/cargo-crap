import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import resolver from "../../npm/crapts/lib/resolve.js";

function npm(args, options) {
  if (process.platform !== "win32") return execFileSync("npm", args, options);
  const cli = join(
    dirname(process.execPath),
    "node_modules/npm/bin/npm-cli.js",
  );
  return execFileSync(process.execPath, [cli, ...args], options);
}

export async function verifyBundle(bundle, identity, trustCache = {}) {
  const { verify } = await import("sigstore");
  await verify(bundle, {
    ...trustCache,
    certificateIdentityURI: identity,
    certificateIssuer: "https://token.actions.githubusercontent.com",
  });
}

async function verifyExisting(
  { dist },
  { integrity, name, version, repo, commit, registry },
  verify,
) {
  if (dist?.integrity !== integrity) throw Error("tarball integrity mismatch");
  if (!dist.attestations?.url) throw Error("missing provenance");
  const endpoint = new URL(dist.attestations.url, registry);
  if (endpoint.origin !== new URL(registry).origin)
    throw Error("unexpected provenance registry");
  const response = await fetch(endpoint);
  if (!response.ok) throw Error("provenance HTTP " + response.status);
  const { attestations } = await response.json();
  const type = "https://slsa.dev/provenance/v1";
  const bundle = attestations.find((a) => a.predicateType === type)?.bundle;
  if (!bundle?.dsseEnvelope?.payload) throw Error("missing SLSA provenance");
  const source = "https://github.com/" + repo;
  // A build from another ref can share this commit but lacks authority to issue the release.
  const ref = "refs/tags/crapts-v" + version;
  await verify(bundle, source + "/.github/workflows/release-crapts.yml@" + ref);
  const stmt = JSON.parse(Buffer.from(bundle.dsseEnvelope.payload, "base64"));
  const def = stmt.predicate?.buildDefinition;
  const sha512 = Buffer.from(
    integrity.slice("sha512-".length),
    "base64",
  ).toString("hex");
  const purl = "pkg:npm/" + name.replace("@", "%40") + "@" + version;
  if (
    stmt.predicateType !== type ||
    !stmt.subject?.some(
      (s) => s.name === purl && s.digest?.sha512 === sha512,
    ) ||
    def?.externalParameters?.workflow?.repository !== source ||
    !def?.resolvedDependencies?.some(
      (d) =>
        d.uri === "git+" + source + "@" + ref && d.digest?.gitCommit === commit,
    )
  ) {
    throw Error("provenance package, repository or commit mismatch");
  }
}

export async function publishRelease(
  {
    stagedDir,
    version,
    commit,
    repo,
    registry = "https://registry.npmjs.org",
    tag = "next",
  },
  {
    publish = (archive, flags) =>
      npm(["publish", archive, ...flags], { stdio: "inherit" }),
    verifyBundle: verify = verifyBundle,
  } = {},
) {
  if (!stagedDir || !version || !commit || !repo)
    throw Error("expected --staged-dir, --version, --commit and --repo");
  const scratch = mkdtempSync(join(tmpdir(), "crapts-publish-"));
  try {
    for (const name of [
      ...resolver.targets.map((t) => t.name),
      "@minikin_/crapts",
    ]) {
      try {
        const dir = resolve(stagedDir, basename(name));
        const manifest = JSON.parse(readFileSync(join(dir, "package.json")));
        if (manifest.name !== name || manifest.version !== version)
          throw Error("staged manifest mismatch");
        const response = await fetch(
          new URL(
            encodeURIComponent(name) + "/" + encodeURIComponent(version),
            registry + "/",
          ),
        );
        if (![200, 404].includes(response.status))
          throw Error("registry HTTP " + response.status);
        const [packed] = JSON.parse(
          npm(
            [
              "pack",
              "--json",
              "--ignore-scripts",
              "--pack-destination",
              scratch,
            ],
            { cwd: dir, encoding: "utf8" },
          ),
        );
        if (response.status === 200) {
          await verifyExisting(
            await response.json(),
            {
              integrity: packed.integrity,
              name,
              version,
              repo,
              commit,
              registry,
            },
            verify,
          );
        } else {
          await publish(
            join(scratch, packed.filename),
            [
              "--provenance",
              "--access",
              "public",
              "--tag",
              tag,
              "--registry",
              registry,
            ],
            name,
          );
        }
      } catch (e) {
        throw Error(name + ": " + e.message, { cause: e });
      }
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const options = Object.fromEntries(
      ["staged-dir", "version", "commit", "repo", "tag", "registry"].map(
        (key) => [key, { type: "string" }],
      ),
    );
    const args = parseArgs({ options }).values;
    await publishRelease({ ...args, stagedDir: args["staged-dir"] });
  } catch (e) {
    console.error(e.message);
    process.exitCode = 1;
  }
}
