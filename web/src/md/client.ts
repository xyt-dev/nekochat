// Main-thread side of the markdown worker: one in-flight request per message, results applied
// in the frame loop, rendered blocks cached per message so virtual scrolling can remount cheaply.
import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";
import { inNextFrame } from "../frame";
import type { MdBlock, MdRequest, MdResponse } from "./protocol";

interface Entry {
  key: string;
  /** Source of `blocks`; `null` until the first response. */
  text: string | null;
  final: boolean;
  blocks: MdBlock[];
  listeners: Set<(blocks: MdBlock[]) => void>;
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
  e = { key, text: null, final: false, blocks: [], listeners: new Set(), inflight: false, want: null };
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
  post({ key: e.key, text, final, base: e.blocks.length });
}

// What each in-flight request rendered, applied with its response.
const sentText = new Map<string, { text: string; final: boolean }>();
/** Remove every account's source, blocks and queued render result on logout/switch. */
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
    const updates = res.blocks.map((next, index) => {
      const previous = e.blocks[res.start + index];
      return next.kind === "graphic" && previous?.kind === "graphic" && next.source === previous.source
        ? previous : next;
    });
    e.blocks = e.blocks.slice(0, res.start).concat(updates);
    e.blocks.length = res.total;
    e.text = sent.text;
    e.final = sent.final;
    e.inflight = false;
    for (const l of e.listeners) l(e.blocks);
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
 * Typed rendered blocks of `text`; `null` until the first response. While streaming,
 * finished blocks stay frozen and only the last block is redone.
 */
export function createMarkdown(
  key: Accessor<string>,
  text: Accessor<string>,
  final: Accessor<boolean>,
): Accessor<MdBlock[] | null> {
  const [blocks, setBlocks] = createSignal<MdBlock[] | null>(null, { equals: false });
  createEffect(() => {
    const e = entry(key());
    const listener = (value: MdBlock[]) => setBlocks(value);
    e.listeners.add(listener);
    setBlocks(e.text === null ? null : e.blocks);
    onCleanup(() => e.listeners.delete(listener));
  });
  createEffect(() => request(key(), text(), final()));
  return blocks;
}
