import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const owned = new Set(["--format", "--output", "--base", "--baseline",
  "--comment-marker", "--commit-ref", "--all-rows"]);

function words(input) {
  const result = [];
  let word = "", quote = "", started = false;
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (!quote && /\s/.test(c)) {
      if (started) result.push(word);
      word = "";
      started = false;
      continue;
    }
    started = true;
    if (quote && c === quote) quote = "";
    else if (!quote && (c === '"' || c === "'")) quote = c;
    else if (c === "\\" && quote !== "'" && (!quote || /["\\$`\n]/.test(input[i + 1]))) {
      if (++i === input.length) throw Error("args: check its quotes and trailing escape");
      if (input[i] !== "\n") word += input[i];
    } else word += c;
  }
  if (quote) throw Error("args: check its quotes");
  if (started) result.push(word);
  return result;
}

export function argumentsFrom(env) {
  for (const key of ["fail-regression", "fail-changed", "fail-above", "comment"]) {
    const value = env["INPUT_" + key.toUpperCase().replaceAll("-", "_")];
    if (!["true", "false"].includes(value)) throw Error(key + " must be true or false, got " + value);
  }
  if (!env.INPUT_COMMENT_MARKER || /[\r\n]/.test(env.INPUT_COMMENT_MARKER))
    throw Error("comment-marker must be one nonempty line");
  const args = words(env.INPUT_ARGS || "");
  for (const word of args) {
    const flag = word.split("=")[0];
    if (owned.has(flag)) throw Error("args cannot set " + flag);
  }
  return args;
}

if (resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    console.log(JSON.stringify(argumentsFrom(process.env)));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 2;
  }
}
