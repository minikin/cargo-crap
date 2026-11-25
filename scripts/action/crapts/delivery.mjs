export function firstLineMatches(body, marker) {
  return typeof body === "string" && body.split(/\r?\n/, 1)[0] === marker;
}

export function jobSummary(report, settings) {
  const summary = report + "\n<details><summary>Effective settings</summary>\n\n```json\n" + settings + "\n```\n</details>\n";
  return Buffer.byteLength(summary) < 1000000 ? summary :
    "The crapts report is too large for the job summary. Read the `report` step output.\n";
}

export function ownedBy(comment, account) {
  const author = comment?.user;
  if (!author || !account?.login) return false;
  if (account.id != null && author.id != null) return author.id === account.id;
  const accountType = account.login.endsWith("[bot]") ? "Bot" : account.type;
  if (accountType && author.type && author.type !== accountType) return false;
  return author.login === account.login ||
    (accountType === "Bot" && author.type === "Bot" && author.login === account.login + "[bot]");
}

// The rule @octokit/graphql uses: GitHub Enterprise Server serves REST under
// /api/v3 and GraphQL at /api/graphql.
export function graphqlUrl(api) {
  return api.replace(/\/api\/v3\/?$/, "/api").replace(/\/$/, "") + "/graphql";
}
