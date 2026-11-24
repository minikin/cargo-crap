import assert from "node:assert/strict";
import { test } from "node:test";
import { firstLineMatches, jobSummary } from "../delivery.mjs";

test("only a complete first marker line matches", () => {
  let state = 14;
  for (let i = 0; i < 500; i++) {
    let marker = "";
    for (let j = 0; j <= i % 45; j++) {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      const point = state % 0x110000;
      marker += [10, 13].includes(point) ? " " : String.fromCodePoint(point);
    }
    for (const body of [marker, marker + "\nbody", marker + "\r\nbody"]) assert.ok(firstLineMatches(body, marker));
    for (const body of [marker + "x\n", "quoted\n" + marker]) assert.ok(!firstLineMatches(body, marker));
  }
});

test("summary byte budget includes settings and markup", () => {
  const settings = '{"threshold":30}\n';
  const unit = "\u00e9";
  const overhead = Buffer.byteLength(jobSummary("", settings));
  for (const length of [4096, 999999, 1000000]) {
    const available = length - overhead;
    const report = unit.repeat(Math.floor(available / 2)) + "x".repeat(available % 2);
    const summary = jobSummary(report, settings);
    if (length < 1000000) {
      assert.equal(Buffer.byteLength(summary), length);
      assert.ok(summary.includes(report) && summary.includes(settings));
    } else {
      assert.equal(summary.trim().split("\n").length, 1);
      assert.match(summary, /report.*output/);
    }
  }
});
