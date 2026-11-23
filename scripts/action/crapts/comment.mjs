import { readFileSync } from "node:fs";
import { firstLineMatches } from "./delivery.mjs";

async function checkResponse(response, operation) {
  if (response.ok) return true;
  const status = operation + ": HTTP " + response.status;
  if (response.status === 403 || response.status === 404) {
    console.log("::warning::" + status + ". Read the report in the job summary.");
    return false;
  }
  const details = await response.json().catch(() => null);
  const message = typeof details?.message === "string" ? details.message.replace(/[\r\n]+/g, " ") : "";
  throw Error(message ? status + ": " + message : status);
}

async function deliver(env) {
  if (!env.CRAPTS_PR_NUMBER || env.INPUT_COMMENT !== "true") return;
  const root = env.CRAPTS_API_URL + "/repos/" + env.GITHUB_REPOSITORY;
  const collection = root + "/issues/" + env.CRAPTS_PR_NUMBER + "/comments";
  const headers = { authorization: "Bearer " + env.CRAPTS_TOKEN, accept: "application/vnd.github+json" };
  const request = (url, options) => fetch(url, { headers, redirect: "error", ...options });
  const marker = env.INPUT_COMMENT_MARKER;
  const body = readFileSync(env.REPORT, "utf8");
  let match;
  for (let page = 1; ; page++) {
    const response = await request(collection + "?per_page=100&page=" + page);
    if (!await checkResponse(response, "List PR comments")) return;
    const comments = await response.json();
    match = comments.find(comment => firstLineMatches(comment.body, marker));
    if (match || comments.length < 100) break;
  }
  const response = await request(match ? root + "/issues/comments/" + match.id : collection, {
    method: match ? "PATCH" : "POST",
    headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ body }),
  });
  await checkResponse(response, "Write PR comment");
}

try { await deliver(process.env); } catch (error) { console.error(error.message); process.exitCode = 1; }
