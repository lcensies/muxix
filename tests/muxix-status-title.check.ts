/**
 * Self-check for `titleFromContent` in muxix-status.ts.
 *
 * Run: node --experimental-strip-types tests/muxix-status-title.check.ts
 */
import assert from "node:assert/strict";

import { titleFromContent } from "../.pi/extensions/muxix-status.ts";

// Plain prompt passes through.
assert.equal(titleFromContent("fix the sidebar collapse"), "fix the sidebar collapse");

// The injected block never reaches the title.
const injected =
  "fix the sidebar collapse\n\n<muxix-inject>\nPrefer OpenSpec for all feature work.\n</muxix-inject>";
const t = titleFromContent(injected);
assert.equal(t, "fix the sidebar collapse");
assert.ok(!t.includes("OpenSpec"));

// Multi-line and control characters collapse to one printable line.
assert.equal(titleFromContent("line one\n\tline\u0007 two"), "line one line two");

// Over-length truncates to the budget.
const long = titleFromContent("x".repeat(200));
assert.equal(long.length, 40);
assert.ok(long.endsWith("\u2026"));

// Block-array content joins its text blocks.
assert.equal(
  titleFromContent([{ type: "text", text: "block one" }, { type: "image" }, { type: "text", text: "block two" }]),
  "block one block two",
);

// Nothing usable -> empty -> caller leaves the pane title alone.
for (const empty of [undefined, null, "", "   \n\t ", [], "<muxix-inject>\nonly injection"]) {
  assert.equal(titleFromContent(empty), "", `expected empty for ${JSON.stringify(empty)}`);
}

// A session name (from /name or a titling extension) is preferred over a raw
// prompt, and passes through the same sanitizing.
assert.equal(titleFromContent("Fix sidebar identity collapse"), "Fix sidebar identity collapse");
assert.equal(titleFromContent("x".repeat(80)).length, 40);

console.log("muxix-status titleFromContent: all checks passed");
