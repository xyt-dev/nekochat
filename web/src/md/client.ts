// Main-thread side of the markdown worker: one in-flight request per message, results applied
// in the frame loop, rendered blocks cached per message so virtual scrolling can remount cheaply.
import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";
import { inNextFrame } from "../frame";
import type { MdRequest, MdResponse } from "./protocol";

interface Entry {
  key: string;
  /** Source of `html`; `null` until the first response. */
  text: string | null;
  final: boolean;
  html: string[];
  listeners: Set<(html: string[]) => void>;
  inflight: boolean;
  want: { text: string; final: boolean } | null;
}

const MAX_ENTRIES = 600;
const entries = new Map<string, Entry>();
let worker = new Worker(new URL("./md.worker.ts", import.meta.url), { type: "module" });
const arrived: MdResponse[] = [];

function post(req: MdRequest) {
  worker.postMessage(req);
}

function entry(key: string): Entry {
  let e = entries.get(key);
  if (e) {
    entries.delete(key);
    entries.set(key, e);
    return e;
  }
  e = { key, text: null, final: false, html: [], listeners: new Set(), inflight: false, want: null };
  entries.set(key, e);
  if (entries.size > MAX_ENTRIES) {
    for (const old of entries.values()) {
      if (entries.size <= MAX_ENTRIES) break;
      if (old.listeners.size > 0 || old.inflight || old === e) continue;
      entries.delete(old.key);
      post({ key: old.key, text: "", final: true, base: 0, drop: true });
    }
  }
  return e;
}

function pump(e: Entry) {
  if (e.inflight || !e.want) return;
  const { text, final } = e.want;
  e.want = null;
  if (text === e.text && final === e.final) return;
  e.inflight = true;
  sentText.set(e.key, { text, final });
  post({ key: e.key, text, final, base: e.html.length });
}

// What each in-flight request rendered, applied with its response.
const sentText = new Map<string, { text: string; final: boolean }>();
/** Remove every account's source, HTML and queued render result on logout/switch. */
export function resetMarkdownCache(): void {
  if (entries.size === 0 && sentText.size === 0 && arrived.length === 0) return;
  worker.terminate();
  worker = new Worker(new URL("./md.worker.ts", import.meta.url), { type: "module" });
  worker.onmessage = receiveMarkdown;
  entries.clear();
  sentText.clear();
  arrived.length = 0;
}

function applyArrived() {
  for (const res of arrived.splice(0)) {
    if (res.dropped) continue;
    const e = entries.get(res.key);
    const sent = sentText.get(res.key);
    sentText.delete(res.key);
    if (!e || !sent) continue;
    e.html = e.html.slice(0, res.start).concat(res.html);
    e.html.length = res.total;
    e.text = sent.text;
    e.final = sent.final;
    e.inflight = false;
    for (const l of e.listeners) l(e.html);
    pump(e);
  }
}

function receiveMarkdown(ev: MessageEvent<MdResponse>) {
  arrived.push(ev.data);
  inNextFrame(applyArrived);
}
worker.onmessage = receiveMarkdown;

/** Asks for `text` rendered under `key`; only the newest wish is kept while one is in flight. */
function request(key: string, text: string, final: boolean) {
  const e = entry(key);
  e.want = { text, final };
  pump(e);
}

/**
 * Rendered HTML blocks of `text`; `null` until the first rendering arrives. A message streaming
 * passes `final = false`; finished blocks stay frozen and only the last one is redone.
 */
export function createMarkdown(
  key: Accessor<string>,
  text: Accessor<string>,
  final: Accessor<boolean>,
): Accessor<string[] | null> {
  const [html, setHtml] = createSignal<string[] | null>(null, { equals: false });
  createEffect(() => {
    const e = entry(key());
    const listener = (h: string[]) => setHtml(h);
    e.listeners.add(listener);
    setHtml(e.text === null ? null : e.html);
    onCleanup(() => e.listeners.delete(listener));
  });
  createEffect(() => request(key(), text(), final()));
  return html;
}
