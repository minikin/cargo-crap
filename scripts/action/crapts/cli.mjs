import { spawnSync } from "node:child_process";
import { closeSync, openSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Every provider's key and endpoint variable. tests/test_contract.py checks
// both lists against the providers registered in cargo-crap-core.
export const PROVIDER_KEYS = ["TYPESAFE_API_KEY", "OPENAI_API_KEY"];
export const PROVIDER_URLS = ["TYPESAFE_BASE_URL", "OPENAI_BASE_URL"];

const UNSET_KEY = /^warning: duplicate triage skipped: (TYPESAFE_API_KEY|OPENAI_API_KEY) is not set; /;

// Windows environment names are case-insensitive, so `typesafe_api_key` reaches
// a child as TYPESAFE_API_KEY. Matching only the upper-case spelling would leak it.
function without(env, names) {
  return Object.fromEntries(Object.entries(env).filter(([name]) => !names.includes(name.toUpperCase())));
}

export function withoutProviderKeys(env) {
  return without(env, PROVIDER_KEYS);
}

// A project command can write a provider URL to $GITHUB_ENV, and later
// composite steps inherit it. The paid run uses only the URLs install.sh saw.
export function withTrustedUrls(env, trusted) {
  const result = without(env, PROVIDER_URLS);
  for (const name of PROVIDER_URLS) if (typeof trusted[name] === "string") result[name] = trusted[name];
  return result;
}

export function trustedUrls(env) {
  const result = {};
  for (const [name, value] of Object.entries(env))
    if (PROVIDER_URLS.includes(name.toUpperCase())) result[name.toUpperCase()] = value;
  return result;
}

export function optionsEnd(args) {
  const at = args.indexOf("--");
  return at < 0 ? args.length : at;
}

export function scanOnly(args) {
  return args.slice(0, optionsEnd(args)).includes("--duplicates");
}

export function withoutOptions(args, flags) {
  const end = optionsEnd(args);
  return args.filter((arg, i) => i >= end || !flags.includes(arg));
}

export function withOptions(args, options) {
  const at = args.indexOf("--");
  return at < 0 ? [...args, ...options] : [...args.slice(0, at), ...options, ...args.slice(at)];
}

export function unpaidStderr(text) {
  return text.split(/(?<=\n)/).filter(line => !UNSET_KEY.test(line)).join("");
}

// Only the head report may spend the triage budget: `paid` passes the step's
// environment with the provider URLs install.sh recorded. Every other run gets
// no provider key, and its "key is not set" warning is dropped.
export function writeReport(cwd, args, destination, label, { paid = null } = {}) {
  const output = openSync(destination, "w");
  try {
    const options = paid
      ? { cwd, env: withTrustedUrls(process.env, paid), stdio: ["ignore", output, "inherit"] }
      : { cwd, env: withoutProviderKeys(process.env), stdio: ["ignore", output, "pipe"], maxBuffer: 256 * 1024 * 1024 };
    const result = spawnSync(process.env.CRAPTS_BINARY, args, options);
    if (result.error) throw result.error;
    if (!paid) process.stderr.write(unpaidStderr(result.stderr.toString()));
    if (result.status !== 0 && result.status !== 1)
      throw Error(label + " crapts run failed with status " + (result.status ?? result.signal));
    return result.status;
  } finally {
    closeSync(output);
  }
}

if (process.argv[2] === "trusted-urls" && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  for (const [name, value] of Object.entries(trustedUrls(process.env))) {
    if (/[\r\n]/.test(value)) throw Error(name + " must be a single-line URL");
    console.log(name + "=" + value);
  }
