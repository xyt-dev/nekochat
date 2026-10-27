import assert from "node:assert/strict";
import { test } from "node:test";
import { localImageUrl } from "../src/localImages.ts";

test("only exact same-origin resource and uploaded background routes are image sources", () => {
  for (const url of ["/api/resources/nekowhale/a1.gif", "/api/resources/nekowhale-gpt/gpt_001.gif", "/api/backgrounds/abc-123.webp"]) assert.equal(localImageUrl(url), url);
  for (const url of ["https://example.org/a.gif", "//example.org/a.gif", "data:image/png;base64,AA", "/api/resources/../a.gif", "/api/resources/x/..", "/api/resources/x/a%2fgif", "/api/resources/x/a.gif?remote=1", "/api/backgrounds/a.svg", "/api/resources/x/a.gif#fragment", ""]) assert.equal(localImageUrl(url), null);
});
