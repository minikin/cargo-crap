export function firstLineMatches(body, marker) {
  return typeof body === "string" && body.split(/\r?\n/, 1)[0] === marker;
}

export function jobSummary(report, settings) {
  const summary = report + "\n<details><summary>Effective settings</summary>\n\n```json\n" + settings + "\n```\n</details>\n";
  return Buffer.byteLength(summary) < 1000000 ? summary :
    "The crapts report is too large for the job summary. Read the `report` step output.\n";
}
