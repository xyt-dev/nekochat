import assert from "node:assert/strict";
import { test } from "node:test";
import { bindSvgAnimationVariables, svgAnimationValues } from "../src/graphics/svgAnimation.ts";

const theme: Record<string, string> = {
  "--nk-accent": "rgb(20 40 60 / 0.5)",
  "--nk-muted": "#667788",
  "--nk-radius-sm": "8px",
};

test("SMIL theme variables retain native scalar, length, and color syntax", () => {
  assert.equal(bindSvgAnimationVariables("var(--nk-accent);var(--nk-muted)", theme), "rgb(20 40 60 / 0.5);#667788");
  assert.equal(bindSvgAnimationVariables("calc(var(--nk-radius-sm) * 2) 0", theme), "calc(8px * 2) 0");
  assert.equal(bindSvgAnimationVariables("var(--nk-accent, color-mix(in srgb, red, blue))", theme), "rgb(20 40 60 / 0.5)");
  assert.equal(bindSvgAnimationVariables("var(--nk-test-alpha, .25)", theme), ".25");
  assert.equal(bindSvgAnimationVariables("var(--nk-test-length, calc(var(--nk-radius-sm) * 2))", theme), "calc(8px * 2)");
});

test("SMIL binding preserves document variables, quoted content, and comments", () => {
  assert.equal(bindSvgAnimationVariables("var(--local, var(--nk-muted))", theme), "var(--local, #667788)");
  assert.equal(bindSvgAnimationVariables("'var(--nk-accent)' /* var(--nk-muted) */ var( /* alias */ --nk-muted )", theme), "'var(--nk-accent)' /* var(--nk-muted) */ #667788");
  assert.equal(bindSvgAnimationVariables("var(--missing, 0.4)", theme), "var(--missing, 0.4)");
  assert.equal(bindSvgAnimationVariables("var(--nk-missing /* gap */, '/* untouched */')", theme), "'/* untouched */'");
});

test("SMIL binding honors CSS escapes in variable functions and names", () => {
  assert.equal(bindSvgAnimationVariables("v\\61 r(--nk-\\6d uted)", theme), "#667788");
  assert.equal(bindSvgAnimationVariables("var(--nk-\\6d\r\nuted)", theme), "#667788");
});

test("SMIL value splitting leaves delimiters inside strings and functions intact", () => {
  assert.deepEqual(svgAnimationValues("red;url('data:image/png;base64,AA;BB');rgb(0 0 0 / .5)"), ["red", "url('data:image/png;base64,AA;BB')", "rgb(0 0 0 / .5)"]);
  assert.deepEqual(svgAnimationValues("'a;\\'b';/* ; */blue;green"), ["'a;\\'b'", "/* ; */blue", "green"]);
});
