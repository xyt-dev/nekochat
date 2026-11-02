import assert from "node:assert/strict";
import { afterAll, afterEach, mock, test } from "bun:test";
import { createEffect, createRoot } from "solid-js";

// Keep real HTTP/SSE parsing; renderer Workers are covered by browser smoke checks.
mock.module("../src/md/client", () => ({ resetMarkdownCache() {} }));
Object.assign(globalThis, {
  window: Object.assign(new EventTarget(), { setTimeout }),
  location: { pathname: "/", search: "" },
  history: { replaceState() {} },
  requestAnimationFrame: (callback: FrameRequestCallback) => { queueMicrotask(() => callback(performance.now())); return 1; },
});
// Import after installing browser globals and replacing the Worker-backed module.
const { store } = await import("../src/store");
const user = { id: "owner", username: "owner", avatar: null, last_character_id: null, last_conversation_id: null };
const parent = { id: "question", parent_id: null, role: "user" as const, content: "draw", attachments: [], reasoning: null, character_id: null, character_name: null, character_avatar: null, status: "complete" as const, metrics: null, created_at: 1 };
const complete = { ...parent, id: "reply", parent_id: parent.id, role: "assistant" as const, content: '```neko-ui\n{"version":1,"items":[{"type":"text","content":"complete graphic"}]}\n```' };
let active = false;
let historyGate: Promise<void> | undefined;
let observationGate: Promise<void> | undefined;
let observed = Promise.withResolvers<void>();
let historyRequested = Promise.withResolvers<void>();
let writes = 0;
const server = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  async fetch(request) {
    if (request.method !== "GET") writes++;
    const path = new URL(request.url).pathname;
    if (path.endsWith("/turns")) {
      observed.resolve();
      await observationGate;
      if (!active) return new Response(null, { status: 204 });
      const snapshot = { node: { ...complete, content: "latest snapshot", status: "generating" }, user_node: parent, elapsed_ms: 100, reasoning_start_ms: null, reasoning_end_ms: null };
      return new Response(new ReadableStream({ start(controller) { controller.enqueue(new TextEncoder().encode(`event: snapshot\ndata: ${JSON.stringify(snapshot)}\n\n`)); request.signal.addEventListener("abort", () => controller.close(), { once: true }); } }), { headers: { "content-type": "text/event-stream" } });
    }
    if (path.endsWith("/fixture")) {
      historyRequested.resolve();
      await historyGate;
      return Response.json({ id: "fixture", title: "test", character_id: null, leaf_id: active ? parent.id : complete.id, nodes: active ? [parent] : [parent, complete], active_turn_id: active ? complete.id : null });
    }
    return Response.json([]);
  },
});
const nativeFetch = globalThis.fetch;
globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => nativeFetch(new URL(String(input), server.url), init)) as typeof fetch;

function staleTurn() {
  store.setUser(user);
  store.setConv({ id: "fixture", title: "test", character_id: null, leaf_id: parent.id, nodes: [parent], active_turn_id: complete.id });
  store.setComposerText("keep my draft");
  const turn = store.createLiveTurn("fixture", parent.id);
  store.setLive({ fixture: turn });
  store.handleTurnEvent(turn, { type: "start", data: { user_node: parent, assistant_node_id: complete.id, character_id: "role", character_name: "role", character_avatar: null } });
  store.handleTurnEvent(turn, { type: "text", data: { t: "```neko-ui\n{\"version\":1" } });
  return turn;
}
function stateReached(check: () => boolean): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  createRoot(dispose => createEffect(() => { if (check()) { dispose(); resolve(); } }));
  return promise;
}
afterEach(() => { store.resetSession(); active = false; historyGate = observationGate = undefined; observed = Promise.withResolvers<void>(); historyRequested = Promise.withResolvers<void>(); writes = 0; });
afterAll(() => { globalThis.fetch = nativeFetch; server.stop(true); });

test("resuming a stale completed reply restores full graphic content and keeps the draft", async () => {
  const stale = staleTurn();
  const observer = stale.observer;
  await store.resumeActiveTurn();
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.path().at(-1)?.content, complete.content);
  assert.equal(store.composerText(), "keep my draft");
  assert.equal(observer.signal.aborted, true);
  assert.equal(writes, 0, "recovery must never start another model request");
});

test("resuming a running reply replaces only its observer and applies the latest snapshot", async () => {
  active = true;
  const stale = staleTurn();
  const observer = stale.observer;
  const resumed = store.resumeActiveTurn();
  await stateReached(() => store.currentLive()?.content() === "latest snapshot");
  assert.equal(observer.signal.aborted, true);
  assert.equal(store.currentLive()?.assistantId(), complete.id);
  assert.equal(store.composerText(), "keep my draft");
  assert.equal(writes, 0);
  await store.openConversation(null, false, true);
  await resumed;
});

test("a resumed history request cannot overwrite another conversation after navigation", async () => {
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise;
  staleTurn();
  const resumed = store.resumeActiveTurn();
  await historyRequested.promise;
  await store.openConversation(null, false, true);
  gate.resolve();
  await resumed;
  assert.equal(store.conv(), null);
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.composerText(), "keep my draft");
  assert.equal(writes, 0);
});

test("late responses from superseded observers cannot erase the recovered reply", async () => {
  const gate = Promise.withResolvers<void>(); observationGate = gate.promise;
  staleTurn();
  const previous = store.resumeActiveTurn();
  await observed.promise;
  observationGate = undefined;
  await store.resumeActiveTurn();
  gate.resolve();
  await previous;
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.path().at(-1)?.content, complete.content);
  assert.equal(writes, 0);
});

test("a response from an old account cannot restore its conversation after logout", async () => {
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise;
  staleTurn();
  const resumed = store.resumeActiveTurn();
  await historyRequested.promise;
  store.resetSession();
  gate.resolve();
  await resumed;
  assert.equal(store.user(), null);
  assert.equal(store.conv(), null);
  assert.equal(store.currentLive(), undefined);
  assert.equal(writes, 0);
});
