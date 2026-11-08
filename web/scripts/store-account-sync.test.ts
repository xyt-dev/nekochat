import assert from "node:assert/strict";
import { afterAll, afterEach, mock, test } from "bun:test";
import { createEffect, createRoot } from "solid-js";
import type { ConversationView, NodeView } from "../src/api";

// Exercise the real account/turn SSE parser and store against isolated HTTP state.
mock.module("../src/md/client", () => ({ resetMarkdownCache() {} }));
Object.assign(globalThis, {
  window: Object.assign(new EventTarget(), { setTimeout }),
  location: { pathname: "/", search: "", hash: "" },
  history: { replaceState() {} },
  requestAnimationFrame: (callback: FrameRequestCallback) => { queueMicrotask(() => callback(performance.now())); return 1; },
});
// Load after replacing Worker-backed dependencies and installing the browser boundary.
const { store } = await import("../src/store");
const owner = { id: "sync-owner", username: "owner", avatar: null, last_character_id: "role", last_conversation_id: null };
const question: NodeView = { id: "question", parent_id: null, role: "user", content: "question", attachments: [], reasoning: null, character_id: null, character_name: null, character_avatar: null, status: "complete", metrics: null, created_at: 1 };
const answer: NodeView = { ...question, id: "answer", parent_id: question.id, role: "assistant", content: "answer", character_id: "role", character_name: "Role" };
const initial = (): ConversationView => ({ id: "chat", title: "Chat", character_id: "role", leaf_id: question.id, nodes: [question], active_turn_id: null });
let views = new Map<string, ConversationView>();
let accountStreams = new Set<ReadableStreamDefaultController<Uint8Array>>();
let turnStreams = new Set<ReadableStreamDefaultController<Uint8Array>>();
let turnGets = 0;
let liveSnapshotContent = "remote live";
let modelPosts = 0;
let accountConnections = 0;
let accountConnected = Promise.withResolvers<void>();
let historyReads = 0;
let expired = false;
let failNextListRead = false;
let activeReads = 0;
let maximumReads = 0;
let historyGate: Promise<void> | undefined;
let historyRequested = Promise.withResolvers<void>();
let postGate: Promise<void> | undefined;
let postRequested = Promise.withResolvers<void>();
let postStatus = 200;
let createGate: Promise<void> | undefined;
let createRequested = Promise.withResolvers<void>();
let connectionMutation: (() => void) | undefined;
const encode = (event: string, data: unknown) => new TextEncoder().encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
function notify(id: string | null) { for (const stream of accountStreams) stream.enqueue(encode("sync", { conversation_id: id })); }
function turnEvent(event: string, data: unknown) { for (const stream of turnStreams) stream.enqueue(encode(event, data)); }
function stream(request: Request, streams: Set<ReadableStreamDefaultController<Uint8Array>>, start?: (controller: ReadableStreamDefaultController<Uint8Array>) => void) {
  let current: ReadableStreamDefaultController<Uint8Array> | undefined;
  return new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      current = controller;
      streams.add(controller);
      request.signal.addEventListener("abort", () => { streams.delete(controller); try { controller.close(); } catch {} }, { once: true });
      start?.(controller);
    },
    cancel() { if (current) streams.delete(current); },
  }), { headers: { "content-type": "text/event-stream" } });
}
const server = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/api/account/login") return Response.json(owner);
    if (path === "/api/account/events") {
      if (expired) return Response.json({ error: "session expired" }, { status: 401 });
      accountConnections++;
      accountConnected.resolve();
      connectionMutation?.(); connectionMutation = undefined;
      return stream(request, accountStreams, controller => controller.enqueue(encode("sync", { conversation_id: null })));
    }
    if (path === "/api/account/preferences") return Response.json(owner);
    if (path === "/api/characters") return Response.json({ default_character_id: "role", characters: [{ id: "role", display_name: "Role", avatar: null, background: null, vision: true }] });
    if (path === "/api/conversations" && request.method === "POST") {
      createRequested.resolve(); await createGate;
      const view = { ...initial(), id: "new-chat", nodes: [], leaf_id: null };
      views.set(view.id, view); notify(view.id);
      return Response.json({ id: view.id, title: view.title, character_id: view.character_id, updated_at: 2 });
    }
    if (path.endsWith("/turns")) {
      if (request.method === "POST") {
        modelPosts++; postRequested.resolve(); await postGate;
        if (postStatus !== 200) return Response.json({ error: postStatus === 409 ? "already generating" : "missing" }, { status: postStatus });
        views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
        return stream(request, turnStreams, controller => controller.enqueue(encode("start", { user_node: question, assistant_node_id: answer.id, character_id: "role", character_name: "Role", character_avatar: null })));
      }
      turnGets++;
      const id = decodeURIComponent(path.split("/").at(-2)!);
      const view = views.get(id);
      if (!view?.active_turn_id) return new Response(null, { status: 204 });
      return stream(request, turnStreams, controller => controller.enqueue(encode("snapshot", { node: { ...answer, id: view.active_turn_id, content: liveSnapshotContent }, user_node: question, elapsed_ms: 100, reasoning_start_ms: null, reasoning_end_ms: null })));
    }
    if (path === "/api/conversations" || path.startsWith("/api/conversations/")) {
      if (path === "/api/conversations" && failNextListRead) {
        failNextListRead = false;
        return Response.json({ error: "temporarily unavailable" }, { status: 503 });
      }
      activeReads++; maximumReads = Math.max(maximumReads, activeReads);
      // Capture before waiting so these are genuinely stale HTTP responses.
      const payload = path === "/api/conversations" ? [...views.values()].map(view => ({ id: view.id, title: view.title, character_id: view.character_id, updated_at: 1 })) : structuredClone(views.get(decodeURIComponent(path.split("/").at(-1)!)));
      if (path !== "/api/conversations") { historyReads++; historyRequested.resolve(); await historyGate; }
      activeReads--;
      return payload ? Response.json(payload) : Response.json({ error: "missing" }, { status: 404 });
    }
    return Response.json({ error: "unexpected route" }, { status: 404 });
  },
});
const nativeFetch = globalThis.fetch;
globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => nativeFetch(new URL(String(input), server.url), init)) as typeof fetch;
function reached(check: () => boolean): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  createRoot(dispose => createEffect(() => { if (check()) { dispose(); resolve(); } }));
  return promise;
}
async function login() {
  views.set("chat", initial());
  await store.authenticate("login", "owner", "password");
  await store.openConversation("chat", false);
  await accountConnected.promise;
  await store.loadConversations();
}
afterEach(() => {
  store.resetSession();
  for (const controller of [...accountStreams, ...turnStreams]) { try { controller.close(); } catch {} }
  accountStreams = new Set(); turnStreams = new Set(); views = new Map();
  turnGets = modelPosts = accountConnections = activeReads = maximumReads = 0;
  historyGate = postGate = createGate = undefined; connectionMutation = undefined;
  historyRequested = Promise.withResolvers<void>(); postRequested = Promise.withResolvers<void>(); createRequested = Promise.withResolvers<void>();
  accountConnected = Promise.withResolvers<void>(); historyReads = 0;
  expired = false;
  failNextListRead = false;
  postStatus = 200;
  liveSnapshotContent = "remote live";
});
afterAll(() => { globalThis.fetch = nativeFetch; server.stop(true); });

test("subscription reconciliation closes the login fetch race without following remote preferences", async () => {
  connectionMutation = () => { views.set("remote", { ...initial(), id: "remote", title: "Created while connecting" }); };
  await login();
  await reached(() => store.conversations().some(item => item.id === "remote"));
  assert.equal(accountConnections, 1);
  assert.equal(store.conv()?.id, "chat");
  assert.equal(maximumReads, 1, "account history and list reads must serialize");
});

test("remote live generation attaches once and terminal history preserves local composer state", async () => {
  await login();
  store.setComposerText("unsent draft");
  const attachment = { id: "upload", name: "image.png", kind: "image" as const, media_type: "image/png", size: 1, url: "/uploads/image.png" };
  store.setComposerAttachments([attachment]);
  store.setEditing({ nodeId: question.id });
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.content() === "remote live");
  const live = store.currentLive();
  notify("chat"); notify("chat");
  views.set("other", { ...initial(), id: "other" }); notify("other");
  await reached(() => store.conversations().some(item => item.id === "other"));
  assert.equal(store.currentLive(), live);
  assert.equal(turnGets, 1);
  turnEvent("text", { t: " continued" });
  await reached(() => store.currentLive()?.content() === "remote live continued");
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer] }); notify("chat");
  await reached(() => store.currentLive() === undefined && store.path().at(-1)?.id === answer.id);
  assert.equal(store.composerText(), "unsent draft");
  assert.deepEqual(store.composerAttachments(), [attachment]);
  assert.deepEqual(store.editing(), { nodeId: question.id });
  assert.equal(modelPosts, 0);
});

test("an in-flight authoritative snapshot cannot overwrite a newer live terminal event", async () => {
  await login();
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.content() === "remote live");
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise; historyRequested = Promise.withResolvers<void>();
  notify("chat"); await historyRequested.promise;
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer] });
  turnEvent("done", { node: answer });
  await reached(() => store.path().at(-1)?.content === "answer");
  gate.resolve(); historyGate = undefined;
  await store.loadConversations();
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.path().at(-1)?.content, "answer");
  assert.equal(turnGets, 1);
});

test("sync during a local POST retains its pending turn and never adds a second observer", async () => {
  await login();
  const gate = Promise.withResolvers<void>(); postGate = gate.promise;
  const sending = store.runTurn({ action: "send", content: "local" });
  await postRequested.promise;
  const pending = store.currentLive();
  views.set("marker", { ...initial(), id: "marker" }); notify(null);
  await reached(() => store.conversations().some(item => item.id === "marker"));
  assert.equal(store.currentLive(), pending);
  gate.resolve();
  await reached(() => store.currentLive()?.started() === true);
  assert.equal(store.currentLive(), pending);
  assert.equal(modelPosts, 1);
  assert.equal(turnGets, 0);
  await store.openConversation(null, false, true); await sending;
});

test("remote branch, clear, and delete apply in place while retaining an unsent draft", async () => {
  await login(); store.setComposerText("draft");
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer] }); notify("chat");
  await reached(() => store.path().at(-1)?.id === answer.id);
  views.set("chat", { ...initial(), leaf_id: question.id, nodes: [question, answer] }); notify("chat");
  await reached(() => store.path().at(-1)?.id === question.id);
  views.set("chat", { ...initial(), leaf_id: null, nodes: [] }); notify("chat");
  await reached(() => store.conv()?.id === "chat" && store.path().length === 0);
  assert.equal(store.composerText(), "draft");
  views.delete("chat"); notify("chat");
  await reached(() => store.conv() === null);
  assert.equal(store.composerText(), "draft");
});

test("foreground reconciliation discovers missed work without an already mounted live turn", async () => {
  await login();
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer] });
  await store.resumeActiveTurn();
  assert.equal(store.path().at(-1)?.id, answer.id);
  assert.equal(modelPosts, 0);
});

test("navigation isolates an old sync snapshot from the next conversation", async () => {
  await login(); store.setComposerText("draft");
  views.set("other", { ...initial(), id: "other", title: "Other" });
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise; historyRequested = Promise.withResolvers<void>();
  notify("chat"); await historyRequested.promise;
  const opening = store.openConversation("other", false, true);
  gate.resolve(); historyGate = undefined;
  await opening; await store.loadConversations();
  assert.equal(store.conv()?.id, "other");
  assert.equal(store.composerText(), "draft");
  assert.equal(store.currentLive(), undefined);
});

test("a logged-out snapshot cannot restore conversation state after the next login", async () => {
  await login();
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise; historyRequested = Promise.withResolvers<void>();
  notify("chat"); await historyRequested.promise;
  store.resetSession(); gate.resolve(); historyGate = undefined;
  await store.authenticate("login", "owner", "password");
  await store.loadConversations();
  assert.equal(store.conv(), null);
  assert.equal(store.currentLive(), undefined);
});

test("local creation keeps its returned conversation when older list reconciliation finishes", async () => {
  await login();
  const gate = Promise.withResolvers<void>(); createGate = gate.promise;
  const creating = store.newConversation(true);
  await createRequested.promise;
  notify(null); gate.resolve();
  assert.equal(await creating, "new-chat");
  await reached(() => store.conversations().some(item => item.id === "new-chat"));
  assert.equal(store.conv()?.id, "new-chat");
  assert.equal(modelPosts, 0);
});

test("reconnecting the account subscription reconciles changes missed while disconnected", async () => {
  await login();
  for (const controller of accountStreams) controller.close();
  accountStreams.clear();
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer] });
  await reached(() => store.path().at(-1)?.id === answer.id);
  assert.equal(accountConnections, 2);
  assert.equal(modelPosts, 0);
});

test("a newer remote generation replaces only the previous generation observer", async () => {
  await login();
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.content() === "remote live");
  const previous = store.currentLive()!;
  views.set("chat", { ...initial(), active_turn_id: "next-answer" }); notify("chat");
  await reached(() => store.currentLive()?.assistantId() === "next-answer" && store.currentLive()?.started() === true);
  assert.equal(previous.observer.signal.aborted, true);
  assert.notEqual(store.currentLive(), previous);
  assert.equal(turnGets, 2);
  assert.equal(modelPosts, 0);
});

test("a persisted terminal snapshot cannot render a duplicate active reply", async () => {
  await login();
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, answer], active_turn_id: answer.id }); notify("chat");
  await reached(() => store.path().at(-1)?.id === answer.id);
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.conv()?.active_turn_id, null);
  assert.equal(turnGets, 0);
});

test("a local creation response cannot navigate back after the user leaves", async () => {
  await login(); store.setComposerText("draft");
  const gate = Promise.withResolvers<void>(); createGate = gate.promise;
  const creating = store.newConversation(true);
  await createRequested.promise;
  await store.openConversation(null, false, true);
  gate.resolve();
  assert.equal(await creating, null);
  assert.equal(store.conv(), null);
  assert.equal(store.composerText(), "draft");
});

test("a burst of invalidations coalesces behind one history read and keeps the latest state", async () => {
  await login();
  const before = historyReads;
  const gate = Promise.withResolvers<void>(); historyGate = gate.promise; historyRequested = Promise.withResolvers<void>();
  notify("chat"); await historyRequested.promise;
  views.set("chat", { ...initial(), title: "Latest", leaf_id: answer.id, nodes: [question, answer] });
  for (let index = 0; index < 50; index++) notify("chat");
  gate.resolve(); historyGate = undefined;
  await reached(() => store.conv()?.title === "Latest" && store.path().at(-1)?.id === answer.id);
  assert.ok(historyReads - before <= 3, "notifications must not queue one GET per event");
  assert.equal(maximumReads, 1);
});

test("an expired account subscription retires its live state and outstanding requests", async () => {
  await login();
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.started() === true);
  const observer = store.currentLive()!.observer;
  expired = true;
  await store.resumeActiveTurn();
  await reached(() => store.user() === null);
  assert.equal(store.conv(), null);
  assert.equal(store.currentLive(), undefined);
  assert.equal(observer.signal.aborted, true);
  assert.equal(modelPosts, 0);
});

test("a transient authoritative GET failure retries without requiring another notification", async () => {
  await login();
  failNextListRead = true;
  views.set("missed", { ...initial(), id: "missed", title: "Recovered" }); notify("missed");
  await reached(() => store.conversations().some(item => item.id === "missed"));
  assert.equal(store.conv()?.id, "chat");
  assert.equal(modelPosts, 0);
  assert.equal(maximumReads, 1);
});

test("an empty upstream failure remains visible after its account terminal reconciliation", async () => {
  await login(); store.setComposerText("draft");
  liveSnapshotContent = "";
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.started() === true);
  const failed = store.currentLive()!;
  views.set("chat", { ...initial(), title: "Failed" });
  turnEvent("error", { message: "upstream rejected the request", node: null });
  for (const controller of turnStreams) controller.close(); turnStreams.clear();
  await reached(() => store.currentLive()?.error() === "upstream rejected the request");
  notify("chat"); await store.loadConversations();
  await store.resumeActiveTurn();
  assert.equal(store.currentLive(), failed);
  assert.equal(store.currentLive()?.error(), "upstream rejected the request");
  assert.equal(store.conv()?.active_turn_id, null);
  assert.equal(store.currentLive()?.content(), "");
  assert.equal(store.composerText(), "draft");
  assert.equal(turnGets, 1);
});

test("account terminal arriving first lets the existing stream deliver an empty failure", async () => {
  await login();
  liveSnapshotContent = "";
  views.set("chat", { ...initial(), active_turn_id: answer.id }); notify("chat");
  await reached(() => store.currentLive()?.started() === true);
  const draining = store.currentLive()!;
  views.set("chat", { ...initial(), title: "Terminal committed" }); notify("chat");
  await reached(() => store.conv()?.title === "Terminal committed");
  await store.loadConversations();
  assert.equal(store.currentLive(), draining);
  assert.equal(draining.observer.signal.aborted, false, "inactive history has no replacement for the pending terminal error");
  turnEvent("error", { message: "empty provider failure", node: null });
  for (const controller of turnStreams) controller.close(); turnStreams.clear();
  await reached(() => store.currentLive()?.error() === "empty provider failure");
  await store.loadConversations();
  assert.equal(store.currentLive()?.error(), "empty provider failure");
  assert.equal(store.currentLive()?.content(), "");
  assert.equal(turnGets, 1);
  assert.equal(modelPosts, 0);
});

test("a losing local POST replays a winner notification deferred while its response was pending", async () => {
  await login(); store.setComposerText("draft");
  const gate = Promise.withResolvers<void>(); postGate = gate.promise; postStatus = 409;
  const sending = store.runTurn({ action: "send", content: "local attempt" });
  await postRequested.promise;
  const pending = store.currentLive();
  views.set("chat", { ...initial(), active_turn_id: "remote-winner" });
  views.set("marker", { ...initial(), id: "marker" }); notify(null);
  await reached(() => store.conversations().some(item => item.id === "marker"));
  await store.loadConversations();
  assert.equal(store.currentLive(), pending);
  gate.resolve();
  assert.equal(await sending, false);
  await reached(() => store.currentLive()?.assistantId() === "remote-winner" && store.currentLive()?.content() === "remote live");
  assert.notEqual(store.currentLive(), pending);
  assert.equal(store.composerText(), "draft");
  assert.equal(modelPosts, 1);
  assert.equal(turnGets, 1);
});

test("a pending POST rejected after remote deletion replays the deferred delete", async () => {
  await login(); store.setComposerText("draft");
  const gate = Promise.withResolvers<void>(); postGate = gate.promise; postStatus = 404;
  const sending = store.runTurn({ action: "send", content: "local attempt" });
  await postRequested.promise;
  views.delete("chat"); views.set("marker", { ...initial(), id: "marker" }); notify(null);
  await reached(() => store.conversations().some(item => item.id === "marker"));
  await store.loadConversations();
  assert.equal(store.conv()?.id, "chat");
  gate.resolve();
  assert.equal(await sending, false);
  await reached(() => store.conv() === null);
  assert.equal(store.currentLive(), undefined);
  assert.equal(store.composerText(), "draft");
  assert.equal(modelPosts, 1);
  assert.equal(turnGets, 0);
});

test("regeneration keeps its live parent path when history still selects the previous sibling", async () => {
  await login();
  const previous = { ...answer, id: "previous-reply", content: "previous answer" };
  views.set("chat", { ...initial(), leaf_id: previous.id, nodes: [question, previous] });
  await store.openConversation("chat", false);
  assert.equal(store.path().at(-1)?.id, previous.id);
  const generating = store.runTurn({ action: "regenerate", node_id: question.id });
  await reached(() => store.currentLive()?.started() === true);
  const live = store.currentLive();
  views.set("chat", { ...initial(), title: "Regenerating", leaf_id: previous.id, nodes: [question, previous], active_turn_id: answer.id });
  notify("chat");
  await reached(() => store.conv()?.title === "Regenerating" && store.conv()?.leaf_id === previous.id);
  assert.deepEqual(store.path().map(node => node.id), [question.id]);
  assert.equal(store.currentLive(), live);
  assert.equal(modelPosts, 1);
  assert.equal(turnGets, 0);
  turnEvent("text", { t: "new answer" });
  await reached(() => live?.content() === "new answer");
  assert.deepEqual(store.path().map(node => node.id), [question.id]);
  views.set("chat", { ...initial(), leaf_id: answer.id, nodes: [question, previous, answer] });
  turnEvent("done", { node: answer });
  for (const controller of turnStreams) controller.close(); turnStreams.clear();
  await generating;
  assert.equal(store.currentLive(), undefined);
  assert.deepEqual(store.path().map(node => node.id), [question.id, answer.id]);
  assert.equal(store.conv()?.nodes.filter(node => node.role === "assistant").length, 2);
});
