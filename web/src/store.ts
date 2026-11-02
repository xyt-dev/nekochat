// App state: conversations, the open conversation tree, role choice and live (streaming) turns.
import { type Accessor, batch, createMemo, createRoot, createSignal, type Setter } from "solid-js";
import {
  ApiError,
  api,
  type Attachment,
  type ConversationSummary,
  type ConversationView,
  type CharactersView,
  type NodeView,
  type PickerCharacter,
  type ProfilePatch,
  type UserPublic,
  resetAccountRequests,
  postTurn,
  observeTurn,
  type TurnAction,
  type TurnEvent,
} from "./api";
import { inNextFrame } from "./frame";
import { indexTree, pathTo, type TreeIndex } from "./tree";
import { resetAccountUi, toast } from "./ui";
import { resetMarkdownCache } from "./md/client";
import { resetGraphicsRuntime } from "./graphics/state";

export type ConnectionState = "connecting" | "streaming" | "reconnecting";

/** A reply being generated; its text is buffered and copied to signals once per frame. */
export interface LiveTurn {
  convId: string;
  started: Accessor<boolean>;
  connectionState: Accessor<ConnectionState>;
  assistantId: Accessor<string>;
  /** User message the reply answers. */
  parentId: Accessor<string | null>;
  characterId: Accessor<string>;
  characterName: Accessor<string>;
  characterAvatar: Accessor<string | null>;
  content: Accessor<string>;
  reasoning: Accessor<string>;
  /** `performance.now()` of the first reasoning delta / first text delta after it. */
  reasoningStart: Accessor<number | null>;
  reasoningEnd: Accessor<number | null>;
  /** Set when the turn failed without a stored reply. */
  error: Accessor<string | null>;
  observer: AbortController;
  stopRequested: boolean;
  stopping: Accessor<boolean>;
  // Write side, used by the event handler only.
  buf: { content: string; reasoning: string; dirty: boolean };
  set: {
    started: Setter<boolean>;
    connectionState: Setter<ConnectionState>;
    assistantId: Setter<string>;
    parentId: Setter<string | null>;
    characterId: Setter<string>;
    characterName: Setter<string>;
    characterAvatar: Setter<string | null>;
    content: Setter<string>;
    reasoning: Setter<string>;
    reasoningStart: Setter<number | null>;
    reasoningEnd: Setter<number | null>;
    error: Setter<string | null>;
    stopping: Setter<boolean>;
  };
}

export interface Editing {
  nodeId: string;
}

function createLiveTurn(convId: string, parentId: string | null): LiveTurn {
  const [started, setStarted] = createSignal(false);
  const [connectionState, setConnectionState] = createSignal<ConnectionState>("connecting");
  const [assistantId, setAssistantId] = createSignal("");
  const [parent, setParent] = createSignal(parentId);
  const [characterId, setCharacterId] = createSignal("");
  const [characterName, setCharacterName] = createSignal("");
  const [characterAvatar, setCharacterAvatar] = createSignal<string | null>(null);
  const [content, setContent] = createSignal("");
  const [reasoning, setReasoning] = createSignal("");
  const [reasoningStart, setReasoningStart] = createSignal<number | null>(null);
  const [reasoningEnd, setReasoningEnd] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [stopping, setStopping] = createSignal(false);
  return {
    convId,
    started,
    connectionState,
    assistantId,
    parentId: parent,
    characterId,
    characterName,
    content,
    characterAvatar,
    reasoning,
    reasoningStart,
    reasoningEnd,
    error,
    observer: new AbortController(),
    stopRequested: false,
    stopping,
    buf: { content: "", reasoning: "", dirty: false },
    set: {
      started: setStarted,
      connectionState: setConnectionState,
      assistantId: setAssistantId,
      parentId: setParent,
      characterId: setCharacterId,
      characterName: setCharacterName,
      content: setContent,
      characterAvatar: setCharacterAvatar,
      reasoning: setReasoning,
      reasoningStart: setReasoningStart,
      reasoningEnd: setReasoningEnd,
      error: setError,
      stopping: setStopping,
    },
  };
}

function createAppStore() {
  const [user, setUser] = createSignal<UserPublic | null>(null);
  const [sessionReady, setSessionReady] = createSignal(false);
  const [sessionVersion, setSessionVersion] = createSignal(0);
  let sessionEpoch = 0;
  let preferenceQueue = Promise.resolve();
  let preferenceEpoch = 0;
  const [conversations, setConversations] = createSignal<ConversationSummary[]>([]);
  const [conv, setConv] = createSignal<ConversationView | null>(null, { equals: false });
  const [loadingConv, setLoadingConv] = createSignal(false);
  const [live, setLive] = createSignal<Record<string, LiveTurn>>({});
  const [nodeErrors, setNodeErrors] = createSignal<Record<string, string>>({});
  const [characters, setCharacters] = createSignal<CharactersView>({ default_character_id: null, characters: [] });
  const [characterId, setCharacterId] = createSignal<string | null>(null);
  const [composerText, setComposerText] = createSignal("");
  const [composerAttachments, setComposerAttachments] = createSignal<Attachment[]>([]);
  const [uploadingAttachments, setUploadingAttachments] = createSignal(0);
  const [editing, setEditing] = createSignal<Editing | null>(null);
  const [treeOpen, setTreeOpen] = createSignal(false);
  const [sidebarOpen, setSidebarOpen] = createSignal(false);
  let uploadDraftVersion = 0;

  const index = createMemo<TreeIndex>(() => indexTree(conv()?.nodes ?? []));
  const path = createMemo(() => pathTo(index(), conv()?.leaf_id ?? null));
  const currentLive = createMemo(() => {
    const c = conv();
    return c ? live()[c.id] : undefined;
  });

  /** A role switch only affects subsequent turns; the message tree stays untouched. */
  const character = createMemo<PickerCharacter | undefined>(() => {
    const list = characters().characters;
    return list.find((item) => item.id === characterId()) ??
      list.find((item) => item.id === characters().default_character_id);
  });

  function resetSession(preserveGraphics = false) {
    if (!preserveGraphics) {
      resetGraphicsRuntime();
      try { sessionStorage.removeItem("nk.graphics.account"); } catch { /* Storage may be unavailable. */ }
    }
    sessionEpoch++;
    setSessionVersion(sessionEpoch);
    resetAccountRequests();
    for (const turn of Object.values(live())) turn.observer.abort();
    resetAccountUi();
    resetMarkdownCache();
    loadSeq++;
    uploadDraftVersion++;
    creatingConversation = null;
    batch(() => {
      setUser(null);
      setConversations([]);
      setConv(null);
      setLive({});
      setNodeErrors({});
      setCharacters({ default_character_id: null, characters: [] });
      setCharacterId(null);
      setComposerText("");
      setComposerAttachments([]);
      setUploadingAttachments(0);
      setEditing(null);
      setLoadingConv(false);
      setSidebarOpen(false);
      setTreeOpen(false);
    });
    history.replaceState(null, "", location.pathname + location.search);
  }

  function persistPreferences() {
    if (!user()) return;
    const epoch = sessionEpoch;
    const version = preferenceEpoch;
    const preferences = { last_character_id: character()?.id ?? null, last_conversation_id: conv()?.id ?? null };
    preferenceQueue = preferenceQueue.catch(() => {}).then(async () => {
      if (epoch !== sessionEpoch || version !== preferenceEpoch || !user()) return;
      try {
        const saved = await api.preferences(preferences);
        if (epoch === sessionEpoch && version === preferenceEpoch) setUser((current) => current ? { ...current, last_character_id: saved.last_character_id, last_conversation_id: saved.last_conversation_id } : null);
      } catch (error) {
        if (epoch === sessionEpoch && version === preferenceEpoch) toast(error instanceof ApiError ? error.message : "保存偏好失败");
      }
    });
  }

  async function enterSession(account: UserPublic, requestedConversation: string | null = null, restoreGraphics = false) {
    let preserveGraphics = false;
    if (restoreGraphics) {
      try { preserveGraphics = sessionStorage.getItem("nk.graphics.account") === account.id; } catch { /* Memory-only preferences cannot survive reload. */ }
    }
    resetSession(preserveGraphics);
    try { sessionStorage.setItem("nk.graphics.account", account.id); } catch { /* Storage may be unavailable. */ }
    setUser(account);
    const epoch = sessionEpoch;
    await Promise.all([loadCharacters(), loadConversations()]);
    if (epoch !== sessionEpoch) return;
    const available = characters().characters;
    setCharacterId(available.some((item) => item.id === account.last_character_id) ? account.last_character_id : characters().default_character_id);
    const id = requestedConversation && conversations().some((item) => item.id === requestedConversation)
      ? requestedConversation : account.last_conversation_id;
    if (id && conversations().some((item) => item.id === id)) await openConversation(id, false);
  }

  async function bootstrapSession() {
    const epoch = sessionEpoch;
    const match = location.hash.match(/^#\/c\/(.+)$/);
    let requested: string | null = null;
    try { requested = match ? decodeURIComponent(match[1]) : null; } catch { /* Invalid location is not an account preference. */ }
    try {
      const account = await api.session();
      if (epoch !== sessionEpoch) return;
      if (account) await enterSession(account, requested, true);
      else resetSession();
    } catch (error) {
      if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "加载登录状态失败");
    } finally {
      setSessionReady(true);
    }
  }

  async function authenticate(mode: "login" | "register", username: string, password: string) {
    resetSession();
    const epoch = sessionEpoch;
    const account = await api.authenticate(mode, username, password);
    if (epoch === sessionEpoch) await enterSession(account);
  }

  async function logout() {
    resetSession();
    setSessionReady(false);
    try { await api.logout(); }
    finally { setSessionReady(true); }
  }

  async function saveProfile(body: ProfilePatch): Promise<void> {
    const epoch = sessionEpoch;
    const current = conv();
    const paused = body.password && current ? live()[current.id] : undefined;
    // Own password changes rotate the cookie. Rebind only after fetch has applied the new cookie.
    const passwordRotation = body.password ? Promise.withResolvers<void>() : null;
    if (passwordRotation) {
      // Retire queued old-cookie writes; newly queued preferences wait for PATCH's cookie.
      preferenceEpoch++;
      preferenceQueue = passwordRotation.promise;
      resetAccountRequests();
    }
    if (paused) {
      paused.observer.abort();
      dropLive(paused);
    }
    try {
      const saved = await api.profile(body);
      if (epoch === sessionEpoch) setUser(saved);
    } finally {
      passwordRotation?.resolve();
      if (passwordRotation && epoch === sessionEpoch) persistPreferences();
      if (paused && current && epoch === sessionEpoch && conv()?.id === current.id) {
        try {
          const view = await api.conversation(current.id);
          if (epoch === sessionEpoch && conv()?.id === current.id) {
            setConv(view);
            if (view.active_turn_id) attachTurn(view);
          }
        } catch (error) {
          if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "重新连接回复失败");
        }
      }
    }
  }

  function chooseCharacter(item: PickerCharacter) {
    setCharacterId(item.id);
    persistPreferences();
  }

  async function loadCharacters() {
    const epoch = sessionEpoch;
    try {
      const list = await api.characters();
      if (epoch === sessionEpoch) setCharacters(list);
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "加载角色列表失败");
    }
  }

  async function loadConversations() {
    const epoch = sessionEpoch;
    try {
      const list = await api.conversations();
      if (epoch !== sessionEpoch) return;
      setConversations(list);
      const c = conv();
      const title = c && list.find((s) => s.id === c.id)?.title;
      if (c && title && title !== c.title) updateConv(c.id, (cv) => (cv.title = title));
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "加载对话列表失败");
    }
  }

  let loadSeq = 0;
  async function openConversation(id: string | null, persist = true, preserveDraft = false) {
    if (!user()) return;
    const seq = ++loadSeq;
    // Navigating detaches observers, never the core-owned jobs.
    for (const turn of Object.values(live())) turn.observer.abort();
    setLive({});
    uploadDraftVersion++;
    if (!preserveDraft) {
      setComposerAttachments([]);
      setComposerText("");
    }
    setEditing(null);
    setTreeOpen(false);
    setSidebarOpen(false);
    if (id === null) {
      setConv(null);
      history.replaceState(null, "", location.pathname + location.search);
      if (persist) persistPreferences();
      return;
    }
    history.replaceState(null, "", `${location.pathname}${location.search}#/c/${encodeURIComponent(id)}`);
    setLoadingConv(true);
    try {
      const view = await api.conversation(id);
      if (seq === loadSeq) {
        setConv(view);
        if (persist) persistPreferences();
        if (view.active_turn_id) attachTurn(view);
      }
    } catch (e) {
      if (seq !== loadSeq) return;
      setConv(null);
      if (e instanceof ApiError && e.status === 404) {
        toast("对话不存在");
        history.replaceState(null, "", location.pathname + location.search);
      } else toast(e instanceof ApiError ? e.message : "加载对话失败");
    } finally {
      if (seq === loadSeq) setLoadingConv(false);
    }
  }

  async function reloadConversation(id: string) {
    if (conv()?.id !== id) return;
    const epoch = sessionEpoch;
    try {
      const view = await api.conversation(id);
      if (epoch === sessionEpoch && conv()?.id === id) setConv(view);
    } catch {
      // The next explicit action reports problems.
    }
  }

  /** Applies a change to the open conversation if it is `id`. */
  function updateConv(id: string, f: (c: ConversationView) => void) {
    const c = conv();
    if (!c || c.id !== id) return;
    f(c);
    setConv(c);
  }

  function addNode(c: ConversationView, node: NodeView) {
    const i = c.nodes.findIndex((n) => n.id === node.id);
    if (i >= 0) c.nodes[i] = node;
    else c.nodes = [...c.nodes, node];
  }

  let creatingConversation: Promise<string | null> | null = null;
  function newConversation(preserveDraft = false): Promise<string | null> {
    if (!user()) return Promise.resolve(null);
    const epoch = sessionEpoch;
    if (creatingConversation) return creatingConversation;
    // Detach from the old chat before the request; immediate sends await this same creation.
    if (!preserveDraft) void openConversation(null);
    creatingConversation = (async () => {
      try {
        const summary = await api.createConversation(character()?.id);
        if (epoch !== sessionEpoch) return null;
        setConversations((list) => [summary, ...list.filter((c) => c.id !== summary.id)]);
        await openConversation(summary.id, true, true);
        return summary.id;
      } catch (e) {
        if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "新建对话失败");
        return null;
      }
    })().finally(() => { if (epoch === sessionEpoch) creatingConversation = null; });
    return creatingConversation;
  }

  async function deleteConversation(id: string) {
    const epoch = sessionEpoch;
    try {
      await api.deleteConversation(id);
      live()[id]?.observer.abort();
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setConversations((list) => list.filter((c) => c.id !== id));
      if (conv()?.id === id) await openConversation(null);
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "删除失败");
    }
  }

  async function clearConversation(id: string) {
    const epoch = sessionEpoch;
    try {
      if (live()[id] && !live()[id].error()) {
        toast("请先停止生成，再清空对话");
        return;
      }
      await api.clearConversation(id);
      live()[id]?.observer.abort();
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [id]: _, ...rest } = l;
        return rest;
      });
      setEditing(null);
      if (conv()?.id === id) setComposerAttachments([]);
      uploadDraftVersion++;
      await reloadConversation(id);
      await loadConversations();
    } catch (e) {
      if (epoch === sessionEpoch) toast(e instanceof ApiError ? e.message : "清空失败");
    }
  }

  /** Moves the leaf; `descend` continues down the newest branch below `nodeId`. */
  async function moveLeaf(nodeId: string, descend: boolean) {
    const c = conv();
    const epoch = sessionEpoch;
    if (!c) return;
    if (live()[c.id] && !live()[c.id].error()) {
      toast("正在生成回复，稍后再切换");
      return;
    }
    try {
      const { leaf_id } = await api.setLeaf(c.id, nodeId, descend);
      if (epoch !== sessionEpoch) return;
      setLive((l) => {
        const { [c.id]: _, ...rest } = l;
        return rest;
      });
      updateConv(c.id, (cv) => (cv.leaf_id = leaf_id));
    } catch (e) {
      if (epoch !== sessionEpoch) return;
      if (e instanceof ApiError && e.status === 409) toast("正在生成回复，稍后再切换");
      else toast(e instanceof ApiError ? e.message : "切换失败");
    }
  }

  function flushLive(turn: LiveTurn) {
    if (!turn.buf.dirty) return;
    turn.buf.dirty = false;
    turn.set.content(turn.buf.content);
    turn.set.reasoning(turn.buf.reasoning);
  }

  /** Feeds one SSE event into a live turn and the open conversation. */
  function handleTurnEvent(turn: LiveTurn, ev: TurnEvent) {
    if (turn.connectionState() !== "streaming") turn.set.connectionState("streaming");
    switch (ev.type) {
      case "start": {
        const { user_node, assistant_node_id, character_id, character_name, character_avatar } = ev.data;
        turn.set.assistantId(assistant_node_id);
        turn.set.characterId(character_id);
        turn.set.characterName(character_name);
        turn.set.characterAvatar(character_avatar);
        if (user_node) turn.set.parentId(user_node.id);
        const parent = turn.parentId();
        updateConv(turn.convId, (c) => {
          if (user_node) addNode(c, user_node);
          c.leaf_id = parent;
          c.character_id = character_id;
        });
        turn.set.started(true);
        updateConv(turn.convId, (c) => { c.active_turn_id = assistant_node_id; });
        if (turn.stopRequested) void requestStop(turn);
        break;
      }
      case "snapshot": {
        const { node, user_node, elapsed_ms, reasoning_start_ms, reasoning_end_ms } = ev.data;
        const before = turn.assistantId();
        if (before && before !== node.id) {
          turn.stopRequested = false;
          turn.set.stopping(false);
        }
        batch(() => {
          turn.set.assistantId(node.id);
          turn.set.parentId(node.parent_id);
          turn.set.characterId(node.character_id ?? "");
          turn.set.characterName(node.character_name ?? "");
          turn.set.characterAvatar(node.character_avatar);
          turn.buf.content = node.content;
          turn.buf.reasoning = node.reasoning ?? "";
          turn.buf.dirty = true;
          flushLive(turn);
          const began = performance.now() - elapsed_ms;
          turn.set.reasoningStart(reasoning_start_ms === null ? null : began + reasoning_start_ms);
          turn.set.reasoningEnd(reasoning_end_ms === null ? null : began + reasoning_end_ms);
          turn.set.started(true);
          updateConv(turn.convId, (c) => {
            if (user_node) addNode(c, user_node);
            c.leaf_id = node.parent_id;
            c.active_turn_id = node.id;
          });
        });
        if (turn.stopRequested) void requestStop(turn);
        break;
      }
      case "reasoning":
        if (turn.reasoningStart() === null) turn.set.reasoningStart(performance.now());
        turn.buf.reasoning += ev.data.t;
        turn.buf.dirty = true;
        inNextFrame(() => flushLive(turn));
        break;
      case "text":
        if (turn.reasoningStart() !== null && turn.reasoningEnd() === null) {
          turn.set.reasoningEnd(performance.now());
        }
        turn.buf.content += ev.data.t;
        turn.buf.dirty = true;
        inNextFrame(() => flushLive(turn));
        break;
      case "done":
        finishTurn(turn, ev.data.node, null);
        break;
      case "error":
        finishTurn(turn, ev.data.node, ev.data.message);
        break;
    }
  }

  function finishTurn(turn: LiveTurn, node: NodeView | null, error: string | null) {
    batch(() => {
      flushLive(turn);
      if (node) {
        if (error) setNodeErrors((m) => ({ ...m, [node.id]: error }));
        updateConv(turn.convId, (c) => {
          addNode(c, node);
          c.leaf_id = node.id;
          c.active_turn_id = null;
        });
        dropLive(turn);
      } else {
        turn.set.error(error ?? "回复中断");
      }
    });
  }

  async function uploadAttachments(files: File[]) {
    if (!user() || files.length === 0) return;
    if (composerAttachments().length + uploadingAttachments() + files.length > 5) {
      toast("每条消息最多 5 个附件");
      return;
    }
    if (files.some((file) => file.size === 0 || file.size > (file.type.startsWith("image/") ? 8 : 16) * 1024 * 1024)) {
      toast("图片最大 8 MiB，文档最大 16 MiB，不能上传空文件");
      return;
    }
    const epoch = sessionEpoch;
    setUploadingAttachments((count) => count + files.length);
    try {
      const id = conv()?.id ?? await newConversation(true);
      if (epoch !== sessionEpoch || !id) return;
      const version = uploadDraftVersion;
      for (const file of files) {
        if (epoch !== sessionEpoch || uploadDraftVersion !== version || conv()?.id !== id) break;
        try {
          const attachment = await api.uploadAttachment(id, file);
          if (epoch === sessionEpoch && conv()?.id === id && uploadDraftVersion === version) setComposerAttachments((list) => [...list, attachment]);
        } catch (error) {
          if (epoch === sessionEpoch) toast(error instanceof ApiError ? error.message : "附件上传失败");
        }
      }
    } finally {
      if (epoch === sessionEpoch) setUploadingAttachments((count) => count - files.length);
    }
  }

  function dropLive(turn: LiveTurn) {
    setLive((l) => {
      if (l[turn.convId] !== turn) return l;
      const { [turn.convId]: _, ...rest } = l;
      return rest;
    });
  }

  function attached(turn: LiveTurn, epoch: number, observer: AbortController): boolean {
    return epoch === sessionEpoch && !observer.signal.aborted && turn.observer === observer && live()[turn.convId] === turn;
  }

  function attachTurn(view: ConversationView) {
    if (!view.active_turn_id) return;
    const turn = createLiveTurn(view.id, view.leaf_id);
    turn.set.assistantId(view.active_turn_id);
    setLive((list) => ({ ...list, [view.id]: turn }));
    void watchTurn(turn, sessionEpoch);
  }

  /** Resume HTTP observation only; core generation and the mounted message keep their identities. */
  async function resumeActiveTurn(): Promise<void> {
    const turn = currentLive();
    if (!user() || loadingConv() || !turn?.started() || turn.error() !== null) return;
    turn.observer.abort();
    turn.observer = new AbortController();
    turn.set.connectionState("reconnecting");
    await watchTurn(turn, sessionEpoch);
  }

  /** Observation retries are GET-only. They can never start a second model call. */
  async function watchTurn(turn: LiveTurn, epoch: number): Promise<void> {
    const observer = turn.observer;
    while (attached(turn, epoch, observer)) {
      let settled = false;
      try {
        await observeTurn(turn.convId, observer.signal, (event) => {
          if (!attached(turn, epoch, observer)) return;
          if (event.type === "done" || event.type === "error") settled = true;
          handleTurnEvent(turn, event);
        });
        if (!attached(turn, epoch, observer) || settled) break;
        turn.set.connectionState("reconnecting");
        // The core may have finished between the history snapshot and the observation request.
        const view = await api.conversation(turn.convId);
        if (!attached(turn, epoch, observer)) break;
        setConv(view);
        if (!view.active_turn_id) {
          dropLive(turn);
          break;
        }
      } catch (error) {
        if (!attached(turn, epoch, observer)) break;
        turn.set.connectionState("reconnecting");
        if (error instanceof ApiError && error.status > 0) {
          dropLive(turn);
          toast(error.message);
          if (error.status === 404) {
            setConv(null);
          }
          break;
        }
      }
      if (attached(turn, epoch, observer)) {
        const { promise, resolve } = Promise.withResolvers<void>();
        setTimeout(resolve, 1000);
        await promise;
      }
    }
    if (epoch === sessionEpoch && !observer.signal.aborted) void loadConversations();
  }

  async function requestStop(turn: LiveTurn): Promise<void> {
    const id = turn.assistantId();
    if (!id) return; // A stop clicked before start is sent as soon as the core identifies the turn.
    try {
      await api.stopTurn(turn.convId, id);
    } catch (error) {
      if (live()[turn.convId] !== turn || turn.observer.signal.aborted) return;
      turn.stopRequested = false;
      turn.set.stopping(false);
      toast(error instanceof ApiError ? error.message : "停止失败");
    }
  }

  async function stopTurn(): Promise<void> {
    const turn = currentLive();
    if (!turn || turn.stopping()) return;
    turn.stopRequested = true;
    turn.set.stopping(true);
    await requestStop(turn);
  }

  /** Starts one core job; losing its HTTP observer must not fabricate an interrupted reply. */
  async function runTurn(action: TurnAction): Promise<boolean> {
    if (!user()) return false;
    const epoch = sessionEpoch;
    const selected = character();
    if (!selected) {
      toast("还没有可用的角色，请先在管理端配置并启用");
      return false;
    }
    let c = conv();
    if (!c) {
      const id = await newConversation();
      if (epoch !== sessionEpoch) return false;
      c = conv();
      if (!id || !c) return false;
    }
    const existing = live()[c.id];
    if (existing && !existing.error()) {
      toast("该对话正在生成回复");
      return false;
    }
    const turn = createLiveTurn(c.id, action.action === "regenerate" ? action.node_id : null);
    const observer = turn.observer;
    setLive((list) => ({ ...list, [turn.convId]: turn }));
    setEditing(null);
    let settled = false;
    try {
      await postTurn(turn.convId, { ...action, character_id: selected.id }, observer.signal, (event) => {
        if (!attached(turn, epoch, observer)) return;
        if (event.type === "done" || event.type === "error") settled = true;
        handleTurnEvent(turn, event);
      });
    } catch (error) {
      if (!attached(turn, epoch, observer)) return true;
      if (!turn.started() && error instanceof ApiError && error.status > 0) {
        dropLive(turn);
        toast(error.message);
        return false;
      }
      turn.set.connectionState("reconnecting");
      if (!turn.started()) {
        try {
          const view = await api.conversation(turn.convId);
          if (!attached(turn, epoch, observer)) return true;
          setConv(view);
          if (!view.active_turn_id) {
            dropLive(turn);
            toast("未确认发送成功，输入已保留；请检查对话后重试");
            return false;
          }
          turn.set.assistantId(view.active_turn_id);
        } catch {
          if (attached(turn, epoch, observer)) {
            dropLive(turn);
            toast("连接不可用，输入已保留");
          }
          return false;
        }
      }
    }
    if (attached(turn, epoch, observer) && !settled) {
      turn.set.connectionState("reconnecting");
      await watchTurn(turn, epoch);
    } else if (epoch === sessionEpoch && !observer.signal.aborted) {
      void loadConversations();
    }
    return true;
  }

  return {
    user,
    setUser,
    sessionReady,
    sessionVersion,
    bootstrapSession,
    authenticate,
    logout,
    saveProfile,
    resetSession,
    conversations,
    conv,
    setConv,
    loadingConv,
    live,
    setLive,
    currentLive,
    nodeErrors,
    characters,
    character,
    chooseCharacter,
    composerText,
    setComposerText,
    composerAttachments,
    setComposerAttachments,
    uploadingAttachments,
    uploadAttachments,
    editing,
    setEditing,
    treeOpen,
    setTreeOpen,
    sidebarOpen,
    setSidebarOpen,
    index,
    path,
    loadCharacters,
    loadConversations,
    openConversation,
    newConversation,
    deleteConversation,
    clearConversation,
    moveLeaf,
    runTurn,
    stopTurn,
    resumeActiveTurn,
    createLiveTurn,
    handleTurnEvent,
  };
}

export const store = createRoot(createAppStore);
