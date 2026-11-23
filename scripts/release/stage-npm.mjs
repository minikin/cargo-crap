import {
  cpSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { archiveName, extractBinary } from "./archives.mjs";
import resolver from "../../npm/crapts/lib/resolve.js";

const source = fileURLToPath(new URL("../../npm/crapts/", import.meta.url));
const template = JSON.parse(readFileSync(join(source, "package.json")));

function writeManifest(dir, manifest) {
  for (const hook of ["preinstall", "install", "postinstall", "prepare"]) {
    if (Object.hasOwn(manifest.scripts ?? {}, hook))
      throw Error(manifest.name + " defines " + hook);
  }
  writeFileSync(
    join(dir, "package.json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );
}

function stage({ archives, out, version }) {
  if (!archives || !out || !version)
    throw Error("expected --archives, --out and --version");
  const inputs = resolver.targets.map((target) => {
    const archive = resolve(archives, archiveName(target));
    if (!existsSync(archive)) throw Error("missing " + archive);
    return { target, archive };
  });
  const output = resolve(out);
  if (existsSync(output) && readdirSync(output).length)
    throw Error(output + " must be empty");
  mkdirSync(dirname(output), { recursive: true });
  const temp = mkdtempSync(output + "-");
  try {
    const deps = {};
    for (const { target, archive } of inputs) {
      const dir = join(temp, basename(target.name));
      mkdirSync(join(dir, "bin"), { recursive: true });
      const binary = join(dir, target.binary);
      extractBinary(archive, binary, target);
      deps[target.name] = version;
      writeManifest(dir, {
        name: target.name,
        version,
        description: "Native crapts for " + target.triple,
        license: template.license,
        repository: template.repository,
        os: [target.platform],
        cpu: [target.arch],
        ...(target.platform === "linux" ? { libc: [target.libc] } : {}),
        files: ["bin/"],
        publishConfig: { access: "public" },
      });
    }
    const wrapper = join(temp, "crapts");
    mkdirSync(wrapper);
    for (const dir of ["lib", "bin"])
      cpSync(join(source, dir), join(wrapper, dir), { recursive: true });
    copyFileSync(join(source, "README.md"), join(wrapper, "README.md"));
    copyFileSync(join(source, "../../LICENSE"), join(wrapper, "LICENSE"));
    writeManifest(wrapper, {
      ...template,
      version,
      optionalDependencies: deps,
    });
    if (existsSync(output)) rmdirSync(output);
    renameSync(temp, output);
  } finally {
    rmSync(temp, { force: true, recursive: true });
  }
}

try {
  const options = Object.fromEntries(
    ["version", "archives", "out"].map((key) => [key, { type: "string" }]),
  );
  stage(parseArgs({ options }).values);
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
}
