export function firstLineMatches(body, marker) {
  return typeof body === "string" && body.split(/\r?\n/, 1)[0] === marker;
}

export function jobSummary(report, settings) {
  const summary = report + "\n<details><summary>Effective settings</summary>\n\n```json\n" + settings + "\n```\n</details>\n";
  return Buffer.byteLength(summary) < 1000000 ? summary :
    "The crapts report is too large for the job summary. Read the `report` step output.\n";
}

// GraphQL can return a bot's login without the [bot] suffix the REST comment
// carries. User names cannot contain brackets, so no person matches that branch.
export function ownedBy(comment, login) {
  const user = comment?.user;
  if (!user || typeof login !== "string" || !login) return false;
  return user.login === login || (user.type === "Bot" && user.login === login + "[bot]");
}

// The rule @octokit/graphql uses: GitHub Enterprise Server serves REST under
// /api/v3 and GraphQL at /api/graphql.
export function graphqlUrl(api) {
  return api.replace(/\/api\/v3\/?$/, "/api").replace(/\/$/, "") + "/graphql";
}
