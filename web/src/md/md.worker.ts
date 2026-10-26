/// <reference lib="webworker" />
// Markdown worker: pulldown-cmark (wasm) splits and renders blocks, then KaTeX and highlight.js
// fill the math and code placeholders. Finished blocks are rendered once; while a message
// streams only its last block is re-rendered, without math or highlighting until it completes.
import init, { render_html, split_blocks } from "../wasm/nekochat_mdwasm.js";
import katex from "katex";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import lua from "highlight.js/lib/languages/lua";
import markdown from "highlight.js/lib/languages/markdown";
import php from "highlight.js/lib/languages/php";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import shell from "highlight.js/lib/languages/shell";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import type { MdRequest, MdResponse } from "./protocol";

const languages = {
  bash, c, cpp, csharp, css, diff, go, ini, java, javascript, json, kotlin, lua, markdown, php,
  python, ruby, rust, shell, sql, swift, typescript, xml, yaml,
};
for (const [name, lang] of Object.entries(languages)) hljs.registerLanguage(name, lang);

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };

function unescapeHtml(s: string): string {
  return s.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[a-z]+);/g, (m, e: string) => {
    if (e[0] === "#") {
      const code = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : m;
    }
    return ENTITIES[e] ?? m;
  });
}

const MATH = /<(span|div) class="nk-math" data-display="([01])">([^<]*)<\/\1>/g;
const CODE = /<pre><code(?: class="language-([^"]*)")?>([^<]*)<\/code><\/pre>/g;

function escapeAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

/** Fills math and code placeholders of a finished block. */
function finish(html: string): string {
  html = html.replace(MATH, (_m, tag: string, display: string, tex: string) => {
    const displayMode = display === "1";
    const out = katex.renderToString(unescapeHtml(tex), { displayMode, throwOnError: false });
    return `<${tag} class="nk-math nk-math-done" data-display="${display}">${out}</${tag}>`;
  });
  return html.replace(CODE, (_m, lang: string | undefined, escaped: string) => {
    const known = lang && hljs.getLanguage(lang) ? lang : undefined;
    const body = known
      ? hljs.highlight(unescapeHtml(escaped), { language: known, ignoreIllegals: true }).value
      : escaped;
    const label = lang ? `<span class="nk-code-lang">${escapeAttr(lang)}</span>` : "";
    const cls = lang ? ` class="hljs language-${escapeAttr(lang)}"` : ` class="hljs"`;
    return `<div class="nk-code">${label}<button class="nk-copy" type="button">复制</button><pre><code${cls}>${body}</code></pre></div>`;
  });
}

interface KeyState {
  texts: string[];
  /** The last sent block was rendered unfinished. */
  partialLast: boolean;
}

// What the main thread holds per key, so unchanged leading blocks are not sent again.
const states = new Map<string, KeyState>();
const MAX_STATES = 400;

function handle(req: MdRequest): MdResponse {
  if (req.drop) {
    states.delete(req.key);
    return { key: req.key, total: 0, start: 0, html: [], dropped: true };
  }
  const texts = split_blocks(req.text);
  const prev = req.base > 0 ? states.get(req.key) : undefined;
  let start = 0;
  if (prev) {
    const limit = Math.min(req.base, prev.texts.length, texts.length);
    while (start < limit && prev.texts[start] === texts[start]) start++;
    // The previously unfinished last block is redone once it is finished or no longer last.
    if (prev.partialLast && start === prev.texts.length) {
      const last = start - 1;
      if (last !== texts.length - 1 || req.final) start = last;
    }
  }
  const html: string[] = [];
  for (let i = start; i < texts.length; i++) {
    const raw = render_html(texts[i]);
    const done = req.final || i < texts.length - 1;
    html.push(done ? finish(raw) : raw);
  }
  states.delete(req.key);
  states.set(req.key, { texts, partialLast: !req.final && texts.length > 0 });
  if (states.size > MAX_STATES) states.delete(states.keys().next().value!);
  return { key: req.key, total: texts.length, start, html };
}

const ready = init();
self.onmessage = async (e: MessageEvent<MdRequest>) => {
  await ready;
  let res: MdResponse;
  try {
    res = handle(e.data);
  } catch (err) {
    states.delete(e.data.key);
    const msg = err instanceof Error ? err.message : String(err);
    res = { key: e.data.key, total: 1, start: 0, html: [`<p class="nk-md-error">渲染失败：${escapeAttr(msg)}</p>`] };
  }
  (self as unknown as DedicatedWorkerGlobalScope).postMessage(res);
};
