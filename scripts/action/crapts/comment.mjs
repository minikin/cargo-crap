import { readFileSync } from "node:fs";
import { firstLineMatches, graphqlUrl, ownedBy } from "./delivery.mjs";

const DEFAULT_LOGIN = "github-actions[bot]";

async function failure(response, operation) {
  const status = operation + ": HTTP " + response.status;
  const details = await response.json().catch(() => null);
  const message = typeof details?.message === "string" ? details.message.replace(/[\r\n]+/g, " ") : "";
  return Error(message ? status + ": " + message : status);
}

async function checkResponse(response, operation) {
  if (response.ok) return true;
  if (response.status === 403 || response.status === 404) {
    console.log("::warning::" + operation + ": HTTP " + response.status + ". Read the report in the job summary.");
    return false;
  }
  throw await failure(response, operation);
}

// A refused lookup is not a refused write: a fork's token may still post, so
// only an unusable token (401 or a server error) stops delivery here.
async function lookup(response, operation, read) {
  if (response.ok) return read(await response.json().catch(() => null));
  if (response.status === 403 || response.status === 404) return null;
  throw await failure(response, operation);
}

async function identity(env, request) {
  if (env.CRAPTS_DEFAULT_TOKEN === "true") return DEFAULT_LOGIN;
  const user = await lookup(await request(env.CRAPTS_API_URL + "/user"), "Identify the token", body => body?.login);
  if (typeof user === "string" && user) return user;
  const viewer = await lookup(await request(graphqlUrl(env.CRAPTS_API_URL), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ query: "query { viewer { login } }" }),
  }), "Identify the token", body => body?.data?.viewer?.login);
  return typeof viewer === "string" && viewer ? viewer : null;
}

async function deliver(env) {
  if (!env.CRAPTS_PR_NUMBER || env.INPUT_COMMENT !== "true") return;
  const root = env.CRAPTS_API_URL + "/repos/" + env.GITHUB_REPOSITORY;
  const collection = root + "/issues/" + env.CRAPTS_PR_NUMBER + "/comments";
  const headers = { authorization: "Bearer " + env.CRAPTS_TOKEN, accept: "application/vnd.github+json" };
  const request = (url, options = {}) =>
    fetch(url, { redirect: "error", ...options, headers: { ...headers, ...options.headers } });
  const marker = env.INPUT_COMMENT_MARKER;
  const body = readFileSync(env.REPORT, "utf8");
  // Undefined until the first marker comment, so a PR without one costs no lookup.
  let login;
  let match;
  for (let page = 1; !match; page++) {
    const response = await request(collection + "?per_page=100&page=" + page);
    if (!await checkResponse(response, "List PR comments")) return;
    const comments = await response.json();
    for (const comment of comments) {
      if (!firstLineMatches(comment.body, marker)) continue;
      if (login === undefined) login = await identity(env, request);
      if (login === null) break;
      if (ownedBy(comment, login)) {
        match = comment;
        break;
      }
    }
    if (login === null || comments.length < 100) break;
  }
  if (login === null)
    console.log("::notice::Could not identify the token's account: posting a new report comment instead of updating one");
  const response = await request(match ? root + "/issues/comments/" + match.id : collection, {
    method: match ? "PATCH" : "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body }),
  });
  await checkResponse(response, "Write PR comment");
}

try { await deliver(process.env); } catch (error) { console.error(error.message); process.exitCode = 1; }
